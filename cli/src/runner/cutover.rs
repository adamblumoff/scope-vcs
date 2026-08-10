use super::{load_runner_config, management, runner_client};
use crate::{
    api::{
        RunnerProtocolCanaryRegistration, advance_runner_protocol_cutover,
        create_runner_protocol_canary, get_runner_protocol_cutover,
    },
    clone::{clone_repo, parse_repo_spec},
    git_repo::GitRepo,
    run,
};
use anyhow::{Context, bail};
use scope_api_contract::{
    AdvanceRunnerProtocolCutoverRequest, CreateRunnerProtocolCanaryRequest,
    RunnerProtocolCutoverResponse,
};
use scope_domain::runs::cutover::{
    RunnerProtocolCanaryPhase, RunnerProtocolCanaryStatus, RunnerProtocolCutoverState,
};
use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf, thread, time::Duration};

const OPERATOR_TOKEN_ENV: &str = "SCOPE_OPERATOR_TOKEN";

pub fn recover(
    name: &str,
    repository: &str,
    max_concurrent_jobs: Option<u8>,
) -> anyhow::Result<()> {
    management::install(name, repository, max_concurrent_jobs)?;
    let config = load_runner_config()?;
    let operator_token = std::env::var(OPERATOR_TOKEN_ENV)
        .with_context(|| format!("{OPERATOR_TOKEN_ENV} is required to operate the cutover"))?;
    let client = runner_client()?;
    let target = parse_repo_spec(repository)?;
    let mut checkout = None;

    loop {
        let snapshot = get_runner_protocol_cutover(&client, &config.api_url, &operator_token)?;
        if snapshot.state == RunnerProtocolCutoverState::V7Open {
            println!("✓ Runner protocol V7 is open");
            return Ok(());
        }

        if let Some(canary) = snapshot.canaries.iter().find(|canary| {
            matches!(
                canary.status,
                RunnerProtocolCanaryStatus::Pending | RunnerProtocolCanaryStatus::Running
            )
        }) {
            if canary.runner_id != config.runner_id {
                bail!(
                    "runner protocol canary {} is already assigned to another runner",
                    canary.run_id
                );
            }
            let reconciled = register_canary(
                &client,
                &config.api_url,
                &operator_token,
                &config.runner_id,
                &canary.run_id,
                canary.phase,
            )?;
            let RunnerProtocolCanaryRegistration::Registered(reconciled) = reconciled else {
                bail!("persisted runner protocol canary run is missing");
            };
            if reconciled.canaries.iter().any(|candidate| {
                candidate.run_id == canary.run_id
                    && matches!(
                        candidate.status,
                        RunnerProtocolCanaryStatus::Pending | RunnerProtocolCanaryStatus::Running
                    )
            }) {
                println!(
                    "Resuming {} canary {}",
                    phase_label(canary.phase),
                    canary.run_id
                );
                run::watch_repository(&canary.run_id, repository)?;
                thread::sleep(Duration::from_secs(1));
            }
            continue;
        }

        let Some(assignment) = next_assignment(&snapshot)? else {
            let opened = advance_runner_protocol_cutover(
                &client,
                &config.api_url,
                &operator_token,
                &AdvanceRunnerProtocolCutoverRequest {
                    state: RunnerProtocolCutoverState::V7Open,
                },
            )?;
            if opened.state != RunnerProtocolCutoverState::V7Open {
                bail!("runner protocol cutover did not open");
            }
            println!("✓ Runner protocol V7 is open");
            return Ok(());
        };

        let request_id = canary_request_id(
            repository,
            &config.runner_id,
            assignment.generation,
            assignment.phase,
        );
        let reserved_run_id = format!("run_{request_id}");
        if let RunnerProtocolCanaryRegistration::Registered(_) = register_canary(
            &client,
            &config.api_url,
            &operator_token,
            &config.runner_id,
            &reserved_run_id,
            assignment.phase,
        )? {
            println!(
                "Recovered {} canary {}",
                phase_label(assignment.phase),
                reserved_run_id
            );
            run::watch_repository(&reserved_run_id, repository)?;
            continue;
        }

        if checkout.is_none() {
            checkout = Some(CanaryCheckout::clone(repository)?);
        }
        let queued = run::queue_from_checkout(
            workflow_name(assignment.phase),
            Some(name),
            &checkout
                .as_ref()
                .expect("cutover checkout initialized")
                .repo,
            &target.owner,
            &target.repo,
            &request_id,
        )?;
        if queued.id() != reserved_run_id {
            bail!("queued runner protocol canary did not preserve its reserved run ID");
        }
        println!("Queued {} canary", phase_label(assignment.phase));
        println!("Run ID: {}", queued.id());
        let registered = register_canary(
            &client,
            &config.api_url,
            &operator_token,
            &config.runner_id,
            queued.id(),
            assignment.phase,
        )?;
        if matches!(registered, RunnerProtocolCanaryRegistration::RunMissing) {
            bail!("queued runner protocol canary could not be registered");
        }
        run::watch_queued(&queued)?;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CanaryAssignment {
    generation: u64,
    phase: RunnerProtocolCanaryPhase,
}

fn next_assignment(
    snapshot: &RunnerProtocolCutoverResponse,
) -> anyhow::Result<Option<CanaryAssignment>> {
    if snapshot
        .canaries
        .iter()
        .any(|canary| canary.status == RunnerProtocolCanaryStatus::Failed)
    {
        return Ok(Some(CanaryAssignment {
            generation: snapshot
                .generation
                .checked_add(1)
                .context("runner protocol canary generation overflow")?,
            phase: RunnerProtocolCanaryPhase::ColdWrite,
        }));
    }
    let phase = [
        RunnerProtocolCanaryPhase::ColdWrite,
        RunnerProtocolCanaryPhase::WarmRead,
        RunnerProtocolCanaryPhase::Evict,
    ]
    .into_iter()
    .find(|phase| {
        !snapshot.canaries.iter().any(|canary| {
            canary.phase == *phase && canary.status == RunnerProtocolCanaryStatus::Succeeded
        })
    });
    Ok(phase.map(|phase| CanaryAssignment {
        generation: snapshot.generation.max(1),
        phase,
    }))
}

fn register_canary(
    client: &reqwest::blocking::Client,
    api_url: &str,
    operator_token: &str,
    runner_id: &str,
    run_id: &str,
    phase: RunnerProtocolCanaryPhase,
) -> anyhow::Result<RunnerProtocolCanaryRegistration> {
    create_runner_protocol_canary(
        client,
        api_url,
        operator_token,
        &CreateRunnerProtocolCanaryRequest {
            runner_id: runner_id.to_string(),
            run_id: run_id.to_string(),
            phase,
        },
    )
}

fn canary_request_id(
    repository: &str,
    runner_id: &str,
    generation: u64,
    phase: RunnerProtocolCanaryPhase,
) -> String {
    let identity = format!(
        "{repository}\0{runner_id}\0{generation}\0{}",
        phase_label(phase)
    );
    let digest = Sha256::digest(identity.as_bytes());
    hex::encode(&digest[..16])
}

fn workflow_name(phase: RunnerProtocolCanaryPhase) -> &'static str {
    match phase {
        RunnerProtocolCanaryPhase::ColdWrite => "v7-canary-cold-write",
        RunnerProtocolCanaryPhase::WarmRead => "v7-canary-warm-read",
        RunnerProtocolCanaryPhase::Evict => "v7-canary-evict",
    }
}

struct CanaryCheckout {
    root: PathBuf,
    repo: GitRepo,
}

impl CanaryCheckout {
    fn clone(repository: &str) -> anyhow::Result<Self> {
        let mut suffix = [0_u8; 16];
        getrandom::fill(&mut suffix)
            .map_err(|error| anyhow::anyhow!("create cutover checkout identity: {error}"))?;
        let root = env::temp_dir().join(format!("scope-runner-cutover-{}", hex::encode(suffix)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&root)
                .context("create private cutover checkout directory")?;
        }
        #[cfg(not(unix))]
        fs::create_dir(&root).context("create cutover checkout directory")?;
        let checkout = root.join("repo");
        if let Err(error) = clone_repo(repository, Some(&checkout)) {
            let _ = fs::remove_dir_all(&root);
            return Err(error);
        }
        Ok(Self {
            root,
            repo: GitRepo { root: checkout },
        })
    }
}

impl Drop for CanaryCheckout {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn phase_label(phase: RunnerProtocolCanaryPhase) -> &'static str {
    match phase {
        RunnerProtocolCanaryPhase::ColdWrite => "cold-write",
        RunnerProtocolCanaryPhase::WarmRead => "warm-read",
        RunnerProtocolCanaryPhase::Evict => "eviction",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_api_contract::RunnerProtocolCanaryResponse;

    fn snapshot(
        canaries: Vec<(RunnerProtocolCanaryPhase, RunnerProtocolCanaryStatus)>,
    ) -> RunnerProtocolCutoverResponse {
        RunnerProtocolCutoverResponse {
            state: RunnerProtocolCutoverState::V7Fenced,
            generation: 1,
            enabled_runner_count: 1,
            canaries: canaries
                .into_iter()
                .map(|(phase, status)| RunnerProtocolCanaryResponse {
                    generation: 1,
                    phase,
                    runner_id: "runner-1".to_string(),
                    run_id: "run-1".to_string(),
                    status,
                })
                .collect(),
        }
    }

    #[test]
    fn cutover_resumes_at_the_first_unfinished_phase() {
        assert_eq!(
            next_assignment(&snapshot(Vec::new())).unwrap(),
            Some(CanaryAssignment {
                generation: 1,
                phase: RunnerProtocolCanaryPhase::ColdWrite,
            })
        );
        assert_eq!(
            next_assignment(&snapshot(vec![(
                RunnerProtocolCanaryPhase::ColdWrite,
                RunnerProtocolCanaryStatus::Succeeded,
            )]))
            .unwrap(),
            Some(CanaryAssignment {
                generation: 1,
                phase: RunnerProtocolCanaryPhase::WarmRead,
            })
        );
        assert_eq!(
            next_assignment(&snapshot(vec![
                (
                    RunnerProtocolCanaryPhase::ColdWrite,
                    RunnerProtocolCanaryStatus::Succeeded,
                ),
                (
                    RunnerProtocolCanaryPhase::WarmRead,
                    RunnerProtocolCanaryStatus::Succeeded,
                ),
                (
                    RunnerProtocolCanaryPhase::Evict,
                    RunnerProtocolCanaryStatus::Succeeded,
                ),
            ]))
            .unwrap(),
            None
        );
    }

    #[test]
    fn failed_generation_restarts_from_cold_write() {
        assert_eq!(
            next_assignment(&snapshot(vec![
                (
                    RunnerProtocolCanaryPhase::ColdWrite,
                    RunnerProtocolCanaryStatus::Succeeded,
                ),
                (
                    RunnerProtocolCanaryPhase::WarmRead,
                    RunnerProtocolCanaryStatus::Failed,
                ),
            ]))
            .unwrap(),
            Some(CanaryAssignment {
                generation: 2,
                phase: RunnerProtocolCanaryPhase::ColdWrite,
            })
        );
    }

    #[test]
    fn canary_run_identity_is_stable_and_scoped() {
        let cold = canary_request_id(
            "owner/repo",
            "runner-1",
            3,
            RunnerProtocolCanaryPhase::ColdWrite,
        );
        assert_eq!(cold.len(), 32);
        assert_eq!(
            cold,
            canary_request_id(
                "owner/repo",
                "runner-1",
                3,
                RunnerProtocolCanaryPhase::ColdWrite,
            )
        );
        assert_ne!(
            cold,
            canary_request_id(
                "owner/repo",
                "runner-1",
                3,
                RunnerProtocolCanaryPhase::WarmRead,
            )
        );
        assert_ne!(
            cold,
            canary_request_id(
                "owner/other",
                "runner-1",
                3,
                RunnerProtocolCanaryPhase::ColdWrite,
            )
        );
    }
}
