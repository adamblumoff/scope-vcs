use super::{
    AttemptConclusionRequest, ClaimRunResponse, RunnerConfig, runner_client, runner_work_root,
};
use crate::api::abandon_attempt;
use anyhow::{Context, bail};
use scope_api_contract::StepConclusionRequest;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

const RECOVERY_CLAIM_FILE: &str = "claim.json";
const RECOVERY_CLAIM_TEMP_FILE: &str = ".claim.json.tmp";
const RECOVERY_PROGRESS_FILE: &str = "progress.json";
const RECOVERY_PROGRESS_TEMP_FILE: &str = ".progress.json.tmp";

#[derive(Clone, Debug)]
pub(super) struct RecoveryClaim {
    pub(super) claim: ClaimRunResponse,
    pub(super) progress: RecoveryProgress,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct RecoveryProgress {
    pub(super) next_log_sequence: u64,
    pub(super) execution_deadline_unix: Option<u64>,
    pub(super) active_step_index: Option<u32>,
    pub(super) active_step_nonce: Option<String>,
    pub(super) active_step_log_bytes: u64,
    pub(super) logs_exhausted: bool,
    pub(super) pending_log_chunk: Option<PendingLogChunk>,
    pub(super) pending_step_conclusion: Option<PendingStepConclusion>,
    pub(super) pending_attempt_conclusion: Option<AttemptConclusionRequest>,
    pub(super) pending_attempt_abandon: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct PendingStepConclusion {
    pub(super) step_index: u32,
    pub(super) conclusion: StepConclusionRequest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct PendingLogChunk {
    pub(super) step_index: u32,
    pub(super) sequence: u64,
    pub(super) start_byte: u64,
    pub(super) end_byte: u64,
    pub(super) text: String,
}

pub(super) struct RecoveryAttempt {
    pub(super) work_dir: PathBuf,
    pub(super) recovery: RecoveryClaim,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContainerState {
    Missing,
    Created,
    Running,
    Exited,
}

pub(super) fn persist_recovery_claim(
    work_dir: &Path,
    claim: &ClaimRunResponse,
) -> anyhow::Result<()> {
    if work_dir.join(RECOVERY_CLAIM_FILE).exists() {
        bail!("runner recovery claim already exists");
    }
    write_recovery_progress(
        work_dir,
        &RecoveryProgress {
            next_log_sequence: 1,
            execution_deadline_unix: None,
            active_step_index: None,
            active_step_nonce: None,
            active_step_log_bytes: 0,
            logs_exhausted: false,
            pending_log_chunk: None,
            pending_step_conclusion: None,
            pending_attempt_conclusion: None,
            pending_attempt_abandon: false,
        },
    )?;
    write_private_atomic_json(
        work_dir,
        RECOVERY_CLAIM_FILE,
        RECOVERY_CLAIM_TEMP_FILE,
        claim,
    )
}

pub(super) fn update_recovery_log_progress(
    work_dir: &Path,
    claim: &ClaimRunResponse,
    step_index: u32,
    next_log_sequence: u64,
    step_log_bytes: u64,
    logs_exhausted: bool,
) -> anyhow::Result<()> {
    let stored = matching_recovery_claim(work_dir, claim)?;
    let mut progress = stored.progress;
    if progress.active_step_index != Some(step_index) {
        bail!("runner recovery log progress does not match the active step");
    }
    if let Some(pending) = &progress.pending_log_chunk
        && (pending.step_index != step_index
            || pending.start_byte != progress.active_step_log_bytes
            || pending.end_byte != step_log_bytes
            || (next_log_sequence != pending.sequence
                && next_log_sequence != pending.sequence.saturating_add(1)))
    {
        bail!("runner recovery log commit does not match the pending chunk");
    }
    progress.next_log_sequence = next_log_sequence;
    progress.active_step_log_bytes = step_log_bytes;
    progress.logs_exhausted = logs_exhausted;
    progress.pending_log_chunk = None;
    write_recovery_progress(work_dir, &progress)
}

pub(super) fn stage_recovery_log_chunk(
    work_dir: &Path,
    claim: &ClaimRunResponse,
    pending: PendingLogChunk,
) -> anyhow::Result<()> {
    let stored = matching_recovery_claim(work_dir, claim)?;
    let mut progress = stored.progress;
    if progress.active_step_index != Some(pending.step_index)
        || progress.next_log_sequence != pending.sequence
        || progress.active_step_log_bytes != pending.start_byte
        || pending.end_byte <= pending.start_byte
        || pending.text.is_empty()
    {
        bail!("runner recovery pending log chunk does not match current progress");
    }
    match &progress.pending_log_chunk {
        Some(existing) if existing != &pending => {
            bail!("runner recovery pending log chunk changed before upload")
        }
        Some(_) => return Ok(()),
        None => progress.pending_log_chunk = Some(pending),
    }
    write_recovery_progress(work_dir, &progress)
}

pub(super) fn mark_recovery_step_started(
    work_dir: &Path,
    claim: &ClaimRunResponse,
    step_index: u32,
    step_nonce: &str,
) -> anyhow::Result<()> {
    if step_nonce.is_empty() {
        bail!("runner recovery step nonce is required");
    }
    let stored = matching_recovery_claim(work_dir, claim)?;
    let mut progress = stored.progress;
    match progress.active_step_index {
        Some(active) if active != step_index => {
            bail!("another workflow step is already active in runner recovery state")
        }
        Some(_) if progress.active_step_nonce.as_deref() == Some(step_nonce) => return Ok(()),
        Some(_) => bail!("active workflow step nonce changed during execution"),
        None => {}
    }
    progress.active_step_index = Some(step_index);
    progress.active_step_nonce = Some(step_nonce.to_string());
    progress.active_step_log_bytes = 0;
    progress.pending_log_chunk = None;
    progress.pending_step_conclusion = None;
    write_recovery_progress(work_dir, &progress)
}

pub(super) fn mark_recovery_step_conclusion_pending(
    work_dir: &Path,
    claim: &ClaimRunResponse,
    step_index: u32,
    conclusion: StepConclusionRequest,
) -> anyhow::Result<()> {
    let stored = matching_recovery_claim(work_dir, claim)?;
    let mut progress = stored.progress;
    if progress.active_step_index != Some(step_index) {
        bail!("runner recovery conclusion does not match the active step");
    }
    if progress.pending_log_chunk.is_some() {
        bail!("workflow step cannot conclude while a log chunk is pending upload");
    }
    let pending = PendingStepConclusion {
        step_index,
        conclusion,
    };
    match &progress.pending_step_conclusion {
        Some(existing) if existing != &pending => {
            bail!("runner recovery step conclusion changed after execution")
        }
        Some(_) => return Ok(()),
        None => progress.pending_step_conclusion = Some(pending),
    }
    write_recovery_progress(work_dir, &progress)
}

pub(super) fn mark_recovery_step_completed(
    work_dir: &Path,
    claim: &ClaimRunResponse,
    step_index: u32,
) -> anyhow::Result<()> {
    let stored = matching_recovery_claim(work_dir, claim)?;
    let mut progress = stored.progress;
    if progress.active_step_index != Some(step_index) {
        bail!("runner recovery completion does not match the active step");
    }
    progress.active_step_index = None;
    progress.active_step_nonce = None;
    progress.active_step_log_bytes = 0;
    progress.pending_log_chunk = None;
    progress.pending_step_conclusion = None;
    write_recovery_progress(work_dir, &progress)
}

pub(super) fn mark_recovery_execution_started(
    work_dir: &Path,
    claim: &ClaimRunResponse,
    execution_deadline_unix: u64,
) -> anyhow::Result<()> {
    let stored = matching_recovery_claim(work_dir, claim)?;
    let mut progress = stored.progress;
    progress.execution_deadline_unix = Some(execution_deadline_unix);
    write_recovery_progress(work_dir, &progress)
}

pub(super) fn mark_recovery_conclusion_pending(
    work_dir: &Path,
    claim: &ClaimRunResponse,
    conclusion: AttemptConclusionRequest,
) -> anyhow::Result<()> {
    let stored = matching_recovery_claim(work_dir, claim)?;
    let mut progress = stored.progress;
    if progress.pending_attempt_abandon {
        bail!("runner recovery cannot conclude and abandon the same attempt");
    }
    match &progress.pending_attempt_conclusion {
        Some(pending) if pending != &conclusion => {
            bail!("runner recovery conclusion changed after execution")
        }
        Some(_) => return Ok(()),
        None => progress.pending_attempt_conclusion = Some(conclusion),
    }
    write_recovery_progress(work_dir, &progress)
}

pub(super) fn mark_recovery_abandon_pending(
    work_dir: &Path,
    claim: &ClaimRunResponse,
) -> anyhow::Result<()> {
    let stored = matching_recovery_claim(work_dir, claim)?;
    let mut progress = stored.progress;
    if progress.pending_attempt_conclusion.is_some() {
        bail!("runner recovery cannot abandon and conclude the same attempt");
    }
    progress.pending_attempt_abandon = true;
    write_recovery_progress(work_dir, &progress)
}

fn matching_recovery_claim(
    work_dir: &Path,
    claim: &ClaimRunResponse,
) -> anyhow::Result<RecoveryClaim> {
    let stored = load_recovery_claim(&work_dir.join(RECOVERY_CLAIM_FILE))?;
    if stored.claim.attempt_id != claim.attempt_id
        || stored.claim.attempt_token != claim.attempt_token
    {
        bail!("runner recovery claim identity changed");
    }
    Ok(stored)
}

fn write_recovery_progress(work_dir: &Path, progress: &RecoveryProgress) -> anyhow::Result<()> {
    write_private_atomic_json(
        work_dir,
        RECOVERY_PROGRESS_FILE,
        RECOVERY_PROGRESS_TEMP_FILE,
        progress,
    )
}

fn write_private_atomic_json(
    work_dir: &Path,
    file_name: &str,
    temp_file_name: &str,
    value: &impl serde::Serialize,
) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(value).context("serialize runner recovery claim")?;
    write_private_atomic_bytes(work_dir, file_name, temp_file_name, &bytes)
}

fn write_private_atomic_bytes(
    work_dir: &Path,
    file_name: &str,
    temp_file_name: &str,
    bytes: &[u8],
) -> anyhow::Result<()> {
    let path = work_dir.join(file_name);
    let temp_path = work_dir.join(temp_file_name);
    if temp_path.exists() {
        fs::remove_file(&temp_path).context("remove interrupted runner recovery claim")?;
    }
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp_path)
            .context("create temporary runner recovery claim")?
    };
    #[cfg(not(unix))]
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)
        .context("create temporary runner recovery claim")?;
    let write_result = (|| {
        file.write_all(bytes)?;
        file.sync_all().context("persist runner recovery claim")
    })();
    if let Err(error) = write_result {
        drop(file);
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    drop(file);
    fs::rename(&temp_path, &path).context("publish runner recovery claim")?;
    #[cfg(unix)]
    fs::File::open(work_dir)
        .context("open runner work directory for recovery claim sync")?
        .sync_all()
        .context("persist runner recovery claim directory")?;
    Ok(())
}

pub(super) fn recover_runner_state(config: &RunnerConfig) -> anyhow::Result<Vec<RecoveryAttempt>> {
    let root = runner_work_root()?;
    if !root.exists() {
        reconcile_runner_containers(&config.runner_id, &BTreeSet::new())?;
        return Ok(Vec::new());
    }
    validate_work_root(&root)?;
    let client = runner_client()?;
    let mut active = Vec::new();
    let mut active_containers = BTreeSet::new();
    for entry in fs::read_dir(&root).context("read runner work root")? {
        let entry = entry.context("read runner work entry")?;
        let file_type = entry.file_type().context("inspect runner work entry")?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let claim_path = entry.path().join(RECOVERY_CLAIM_FILE);
        if !claim_path.is_file() {
            continue;
        }
        if !entry.path().join(RECOVERY_PROGRESS_FILE).is_file() {
            let claim = load_claim(&claim_path)?;
            let container_name = container_name(&claim.attempt_id);
            if !super::supervisor::terminate_container(&container_name) {
                bail!("could not confirm incomplete Scope container {container_name} was removed");
            }
            abandon_claim(&client, config, &claim)?;
            fs::remove_dir_all(entry.path()).context("remove incomplete runner recovery state")?;
            continue;
        }
        let recovery = load_recovery_claim(&claim_path)?;
        let container_name = container_name(&recovery.claim.attempt_id);
        let state = container_state(&container_name)?;
        match state {
            ContainerState::Running | ContainerState::Exited => {
                active_containers.insert(container_name);
                active.push(RecoveryAttempt {
                    work_dir: entry.path(),
                    recovery,
                });
            }
            ContainerState::Created => {
                if recovery.progress.pending_attempt_conclusion.is_some()
                    || recovery.progress.pending_step_conclusion.is_some()
                    || recovery.progress.pending_attempt_abandon
                {
                    active_containers.insert(container_name);
                    active.push(RecoveryAttempt {
                        work_dir: entry.path(),
                        recovery,
                    });
                    continue;
                }
                if !super::supervisor::terminate_container(&container_name) {
                    bail!(
                        "could not confirm unstarted Scope container {container_name} was removed"
                    );
                }
                abandon_recovery_claim(&client, config, &recovery)?;
                fs::remove_dir_all(entry.path())
                    .context("remove unstarted runner recovery state")?;
            }
            ContainerState::Missing => {
                if recovery.progress.pending_attempt_conclusion.is_some()
                    || recovery.progress.pending_step_conclusion.is_some()
                    || recovery.progress.pending_attempt_abandon
                {
                    active.push(RecoveryAttempt {
                        work_dir: entry.path(),
                        recovery,
                    });
                } else {
                    abandon_recovery_claim(&client, config, &recovery)?;
                    fs::remove_dir_all(entry.path()).context("remove interrupted runner work")?;
                }
            }
        }
    }
    reconcile_runner_containers(&config.runner_id, &active_containers)?;
    remove_unclaimed_work(&root, &active)?;
    Ok(active)
}

fn load_recovery_claim(path: &Path) -> anyhow::Result<RecoveryClaim> {
    let claim = load_claim(path)?;
    let progress_path = path.with_file_name(RECOVERY_PROGRESS_FILE);
    let progress: RecoveryProgress = serde_json::from_slice(
        &fs::read(&progress_path)
            .with_context(|| format!("read recovery progress {}", progress_path.display()))?,
    )
    .with_context(|| format!("parse recovery progress {}", progress_path.display()))?;
    if progress.next_log_sequence == 0 {
        bail!("runner recovery log sequence must be positive");
    }
    if progress.active_step_index.is_some() != progress.active_step_nonce.is_some() {
        bail!("runner recovery active step identity is incomplete");
    }
    if progress.pending_attempt_abandon && progress.pending_attempt_conclusion.is_some() {
        bail!("runner recovery attempt outcome is ambiguous");
    }
    Ok(RecoveryClaim { claim, progress })
}

fn load_claim(path: &Path) -> anyhow::Result<ClaimRunResponse> {
    serde_json::from_slice(
        &fs::read(path).with_context(|| format!("read recovery claim {}", path.display()))?,
    )
    .with_context(|| format!("parse recovery claim {}", path.display()))
}

fn abandon_recovery_claim(
    client: &reqwest::blocking::Client,
    config: &RunnerConfig,
    recovery: &RecoveryClaim,
) -> anyhow::Result<()> {
    abandon_claim(client, config, &recovery.claim)
}

fn abandon_claim(
    client: &reqwest::blocking::Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
) -> anyhow::Result<()> {
    abandon_attempt(
        client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
    )
}

fn container_name(attempt_id: &str) -> String {
    format!("scope-{attempt_id}")
}

fn container_state(container_name: &str) -> anyhow::Result<ContainerState> {
    let output = Command::new("docker")
        .args([
            "container",
            "inspect",
            "--format={{.State.Running}} {{.State.StartedAt}}",
            container_name,
        ])
        .output()
        .context("inspect interrupted Scope runner container")?;
    if output.status.success() {
        return parse_container_state(&String::from_utf8_lossy(&output.stdout));
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    if stderr.contains("no such") {
        return Ok(ContainerState::Missing);
    }
    bail!(
        "inspect interrupted Scope runner container: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn parse_container_state(value: &str) -> anyhow::Result<ContainerState> {
    let (running, started_at) = value
        .trim()
        .split_once(' ')
        .context("Docker container state is incomplete")?;
    if started_at.starts_with("0001-01-01") {
        return Ok(ContainerState::Created);
    }
    match running {
        "true" => Ok(ContainerState::Running),
        "false" => Ok(ContainerState::Exited),
        _ => bail!("Docker container running state is invalid"),
    }
}

fn reconcile_runner_containers(
    runner_id: &str,
    active_containers: &BTreeSet<String>,
) -> anyhow::Result<()> {
    let output = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--format={{.Names}}",
            "--filter",
            &format!("label=scope.runner-id={runner_id}"),
        ])
        .output()
        .context("list stale Scope runner containers")?;
    if !output.status.success() {
        bail!(
            "list stale Scope runner containers: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    for container_name in String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        if active_containers.contains(container_name) {
            continue;
        }
        if !super::supervisor::terminate_container(container_name) {
            bail!("could not confirm stale Scope container {container_name} was removed");
        }
    }
    Ok(())
}

fn remove_unclaimed_work(root: &Path, active: &[RecoveryAttempt]) -> anyhow::Result<()> {
    let active_paths = active
        .iter()
        .map(|attempt| attempt.work_dir.clone())
        .collect::<BTreeSet<_>>();
    for entry in fs::read_dir(root).context("read runner work root")? {
        let entry = entry.context("read runner work entry")?;
        let path = entry.path();
        if active_paths.contains(&path) {
            continue;
        }
        let file_type = entry.file_type().context("inspect runner work entry")?;
        if file_type.is_dir() && !file_type.is_symlink() {
            fs::remove_dir_all(&path)
                .with_context(|| format!("remove stale runner work {}", path.display()))?;
        } else {
            fs::remove_file(&path)
                .with_context(|| format!("remove stale runner work {}", path.display()))?;
        }
    }
    Ok(())
}

fn validate_work_root(root: &Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(root).context("inspect runner work root")?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        bail!(
            "runner work root is not a real directory: {}",
            root.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_state_distinguishes_created_from_started_containers() {
        assert_eq!(
            parse_container_state("false 0001-01-01T00:00:00Z").unwrap(),
            ContainerState::Created
        );
        assert_eq!(
            parse_container_state("true 2026-07-29T10:00:00Z").unwrap(),
            ContainerState::Running
        );
        assert_eq!(
            parse_container_state("false 2026-07-29T10:00:00Z").unwrap(),
            ContainerState::Exited
        );
        assert!(parse_container_state("unknown 2026-07-29T10:00:00Z").is_err());
    }
}
