use crate::{
    api::{
        RunStreamEvent, api_url, cancel_run, create_manual_run, http_client_builder, retry_run,
        stream_run_events,
    },
    git_repo::{GitRepo, ensure_git_repo_ready, head_oid, warn_if_dirty_working_tree},
    git_transport::{ScopeRemote, select_scope_fetch_remote},
    login::session_from_cache_or_browser,
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_api_contract::{CreateManualRunQuery, RunResponse};
use scope_domain::runs::run::RunState;
use std::{env, fs, path::PathBuf, process::Command, thread, time::Duration};

pub fn start(
    workflow: &str,
    runner: Option<&str>,
    remote: Option<&str>,
    no_watch: bool,
) -> anyhow::Result<()> {
    let repo = ensure_git_repo_ready("scope run")?;
    warn_if_dirty_working_tree(&repo)?;
    let api_url = api_url();
    let target = scope_target(&repo, &api_url, remote)?;
    let git_oid = head_oid(&repo)?;
    let request_id = random_request_id()?;
    let bundle = create_bundle(&repo, &request_id)?;
    let client = run_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    println!("Uploading first-push snapshot for {}", short_oid(&git_oid));
    let run = create_manual_run(
        &client,
        &api_url,
        &session.token,
        &target.owner,
        &target.repo,
        &CreateManualRunQuery {
            workflow: workflow.to_string(),
            git_oid,
            request_id,
            runner: runner.map(str::to_string),
        },
        bundle,
    )?;
    println!(
        "Queued {} on {}",
        run.workflow_name,
        run.desired_runner.as_deref().unwrap_or("any runner")
    );
    println!("Run ID: {}", run.id);
    if no_watch {
        return Ok(());
    }
    watch_run(&client, &api_url, &session.token, &target, &run.id)
}

pub fn watch(run_id: &str, remote: Option<&str>) -> anyhow::Result<()> {
    let repo = ensure_git_repo_ready("scope run watch")?;
    let api_url = api_url();
    let target = scope_target(&repo, &api_url, remote)?;
    let client = run_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    watch_run(&client, &api_url, &session.token, &target, run_id)
}

pub fn cancel(run_id: &str, remote: Option<&str>) -> anyhow::Result<()> {
    let repo = ensure_git_repo_ready("scope run cancel")?;
    let api_url = api_url();
    let target = scope_target(&repo, &api_url, remote)?;
    let client = run_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    let run = cancel_run(
        &client,
        &api_url,
        &session.token,
        &target.owner,
        &target.repo,
        run_id,
    )?;
    println!(
        "Cancellation requested for {} · {}",
        run.id,
        state_label(run.state)
    );
    Ok(())
}

pub fn retry(run_id: &str, remote: Option<&str>, no_watch: bool) -> anyhow::Result<()> {
    let repo = ensure_git_repo_ready("scope run retry")?;
    let api_url = api_url();
    let target = scope_target(&repo, &api_url, remote)?;
    let client = run_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    let run = retry_run(
        &client,
        &api_url,
        &session.token,
        &target.owner,
        &target.repo,
        run_id,
    )?;
    println!("Requeued {} · attempt {}", run.id, run.attempt_number + 1);
    if no_watch {
        return Ok(());
    }
    watch_run(&client, &api_url, &session.token, &target, run_id)
}

fn watch_run(
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: &ScopeRemote,
    run_id: &str,
) -> anyhow::Result<()> {
    let mut cursor = 0;
    loop {
        let mut terminal = None;
        stream_run_events(
            client,
            api_url,
            session_token,
            &target.owner,
            &target.repo,
            run_id,
            cursor,
            |event| {
                match event {
                    RunStreamEvent::Log(log) => {
                        cursor = log.position;
                        print!("{}", log.text);
                    }
                    RunStreamEvent::Status(run) if run.state.is_terminal() => terminal = Some(run),
                    RunStreamEvent::Status(_) => {}
                }
                Ok(terminal.is_none())
            },
        )?;
        if let Some(run) = terminal {
            print_terminal(&run);
            return if run.state == RunState::Succeeded {
                Ok(())
            } else {
                bail!("run {} {}", run.id, state_label(run.state))
            };
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn print_terminal(run: &RunResponse) {
    println!(
        "\nRun {} · {} · {}",
        state_label(run.state),
        short_oid(&run.git_oid),
        run.desired_runner.as_deref().unwrap_or("any runner")
    );
    if run.logs_truncated {
        eprintln!("Warning: this run exceeded the stored log limit; earlier output was truncated.");
    }
}

fn scope_target(
    repo: &GitRepo,
    api_url: &str,
    remote: Option<&str>,
) -> anyhow::Result<ScopeRemote> {
    let remote = select_scope_fetch_remote(repo, api_url, remote)?;
    ScopeRemote::parse(
        api_url,
        &remote,
        &crate::git_repo::git_remote_fetch_url(repo, &remote)?,
    )
}

fn create_bundle(repo: &GitRepo, request_id: &str) -> anyhow::Result<Vec<u8>> {
    let temp = BundleTemp::new(request_id)?;
    let bundle_path = temp.path.join("source.bundle");
    let output = Command::new("git")
        .current_dir(&repo.root)
        .args(["bundle", "create"])
        .arg(&bundle_path)
        .arg("HEAD")
        .output()
        .context("create exact Git bundle for Scope run")?;
    if !output.status.success() {
        bail!(
            "create exact Git bundle for Scope run: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bundle_path, fs::Permissions::from_mode(0o600))
            .context("secure exact Git bundle for Scope run")?;
    }
    fs::read(bundle_path).context("read exact Git bundle for Scope run")
}

fn random_request_id() -> anyhow::Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("generate run request id: {error}"))?;
    Ok(hex::encode(bytes))
}

fn run_client() -> anyhow::Result<Client> {
    http_client_builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("build run HTTP client")
}

fn state_label(state: RunState) -> &'static str {
    match state {
        RunState::Queued => "queued",
        RunState::Leased => "leased",
        RunState::Running => "running",
        RunState::Succeeded => "succeeded",
        RunState::Failed => "failed",
        RunState::Canceled => "canceled",
        RunState::Lost => "lost",
    }
}

fn short_oid(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

struct BundleTemp {
    path: PathBuf,
}

impl BundleTemp {
    fn new(request_id: &str) -> anyhow::Result<Self> {
        let path = env::temp_dir().join(format!("scope-run-upload-{request_id}"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .context("create private run upload directory")?;
        }
        #[cfg(not(unix))]
        fs::create_dir(&path).context("create run upload directory")?;
        Ok(Self { path })
    }
}

impl Drop for BundleTemp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_labels_and_oids_are_stable() {
        assert_eq!(state_label(RunState::Canceled), "canceled");
        assert_eq!(short_oid("1234567890"), "1234567");
        assert_eq!(short_oid("short"), "short");
    }
}
