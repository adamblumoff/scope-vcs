use crate::{
    api::{
        RunStreamEvent, api_url, cancel_run, create_manual_run, http_client_builder, retry_run,
        run_jobs, stream_run_events,
    },
    clone::parse_repo_spec,
    git_repo::{GitRepo, ensure_git_repo_ready, head_oid, warn_if_dirty_working_tree},
    git_transport::{ScopeRemote, select_scope_fetch_remote},
    login::session_from_cache_or_browser,
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_api_contract::{CreateManualRunQuery, RunResponse, RunRunnerSelection};
use scope_domain::runs::run::{RunJobState, RunState};
use std::{env, fs, path::PathBuf, process::Command, thread, time::Duration};

const MAX_PARTIAL_JOB_LINE_BYTES: usize = 8 * 1_024;

pub fn start(
    workflow: &str,
    runner: Option<&str>,
    remote: Option<&str>,
    no_watch: bool,
) -> anyhow::Result<()> {
    let queued = queue(workflow, runner, remote)?;
    print_queued(&queued.run);
    if no_watch {
        return Ok(());
    }
    watch_queued(&queued)
}

pub(crate) struct QueuedRun {
    client: Client,
    api_url: String,
    session_token: String,
    owner: String,
    repo: String,
    run: RunResponse,
}

impl QueuedRun {
    pub(crate) fn id(&self) -> &str {
        &self.run.id
    }
}

pub(crate) fn queue(
    workflow: &str,
    runner: Option<&str>,
    remote: Option<&str>,
) -> anyhow::Result<QueuedRun> {
    let repo = ensure_git_repo_ready("scope run")?;
    warn_if_dirty_working_tree(&repo)?;
    let api_url = api_url();
    let target = scope_target(&repo, &api_url, remote)?;
    queue_from_checkout(
        workflow,
        runner,
        &repo,
        &target.owner,
        &target.repo,
        &random_request_id()?,
    )
}

pub(crate) fn queue_from_checkout(
    workflow: &str,
    runner: Option<&str>,
    checkout: &GitRepo,
    owner: &str,
    repo: &str,
    request_id: &str,
) -> anyhow::Result<QueuedRun> {
    let api_url = api_url();
    let git_oid = head_oid(checkout)?;
    let bundle = create_bundle(checkout, request_id)?;
    let client = run_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    println!("Uploading first-push snapshot for {}", short_oid(&git_oid));
    let run = create_manual_run(
        &client,
        &api_url,
        &session.token,
        owner,
        repo,
        &CreateManualRunQuery {
            workflow: workflow.to_string(),
            git_oid,
            request_id: request_id.to_string(),
            runner: runner.map(str::to_string),
        },
        bundle,
    )?;
    Ok(QueuedRun {
        client,
        api_url,
        session_token: session.token,
        owner: owner.to_string(),
        repo: repo.to_string(),
        run,
    })
}

fn print_queued(run: &RunResponse) {
    println!(
        "Queued {} on {}",
        run.workflow_name,
        runner_selection_label(&run.runner_selection)
    );
    println!("Run ID: {}", run.id);
}

pub(crate) fn watch_queued(queued: &QueuedRun) -> anyhow::Result<()> {
    watch_run(
        &queued.client,
        &queued.api_url,
        &queued.session_token,
        &queued.owner,
        &queued.repo,
        &queued.run.id,
    )
}

pub fn watch(run_id: &str, remote: Option<&str>) -> anyhow::Result<()> {
    let repo = ensure_git_repo_ready("scope run watch")?;
    let api_url = api_url();
    let target = scope_target(&repo, &api_url, remote)?;
    let client = run_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    watch_run(
        &client,
        &api_url,
        &session.token,
        &target.owner,
        &target.repo,
        run_id,
    )
}

pub(crate) fn watch_repository(run_id: &str, repository: &str) -> anyhow::Result<()> {
    let target = parse_repo_spec(repository)?;
    let api_url = api_url();
    let client = run_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    watch_run(
        &client,
        &api_url,
        &session.token,
        &target.owner,
        &target.repo,
        run_id,
    )
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
    println!("Requeued {}", run.id);
    if no_watch {
        return Ok(());
    }
    watch_run(
        &client,
        &api_url,
        &session.token,
        &target.owner,
        &target.repo,
        run_id,
    )
}

fn watch_run(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    run_id: &str,
) -> anyhow::Result<()> {
    let mut cursor = 0;
    let mut line_buffers = JobLineBuffers::default();
    loop {
        let mut terminal = None;
        let stream_result = stream_run_events(
            client,
            api_url,
            session_token,
            owner,
            repo,
            run_id,
            cursor,
            |event| {
                match event {
                    RunStreamEvent::Log(log) => {
                        if advance_log_cursor(&mut cursor, log.position) {
                            print_job_lines(line_buffers.push(&log.job_key, &log.text));
                        }
                    }
                    RunStreamEvent::Status(run) if run.state.is_terminal() => terminal = Some(run),
                    RunStreamEvent::Status(_) => {}
                }
                Ok(terminal.is_none())
            },
        );
        flush_partial_lines_on_stream_error(stream_result, &mut line_buffers)?;
        if let Some(run) = terminal {
            print_job_lines(line_buffers.finish());
            let jobs = run_summary_client().and_then(|summary_client| {
                run_jobs(
                    &summary_client,
                    api_url,
                    session_token,
                    owner,
                    repo,
                    run_id,
                )
            });
            match jobs {
                Ok(jobs) => print_terminal(&run, Some(&jobs)),
                Err(error) => {
                    print_terminal(&run, None);
                    eprintln!("Warning: job summary could not be loaded: {error:#}");
                }
            }
            return if run.state == RunState::Succeeded {
                Ok(())
            } else {
                bail!("run {} {}", run.id, state_label(run.state))
            };
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn advance_log_cursor(cursor: &mut u64, position: u64) -> bool {
    if position <= *cursor {
        return false;
    }
    *cursor = position;
    true
}

fn print_terminal(run: &RunResponse, jobs: Option<&[crate::api::WatchedRunJob]>) {
    println!(
        "\nRun {} · {} · {}",
        state_label(run.state),
        short_oid(&run.git_oid),
        runner_selection_label(&run.runner_selection)
    );
    if run.logs_truncated {
        eprintln!("Warning: this run exceeded the stored log limit; earlier output was truncated.");
    }
    if let Some(jobs) = jobs {
        println!("Jobs:");
        for job in jobs {
            println!("  {} · {}", job.key, job_state_label(job.state));
        }
    }
}

#[derive(Default)]
struct JobLineBuffers {
    active_job: Option<String>,
    partial: String,
}

impl JobLineBuffers {
    fn push(&mut self, job: &str, text: &str) -> Vec<String> {
        let mut lines = Vec::new();
        if self
            .active_job
            .as_deref()
            .is_some_and(|active| active != job)
        {
            lines.extend(self.finish());
        }
        self.active_job.get_or_insert_with(|| job.to_string());
        self.partial.push_str(text);
        while let Some(end) = job_line_end(&self.partial) {
            let line = self.partial.drain(..end).collect::<String>();
            lines.push(format!("[{job}] {}\n", line.trim_end_matches(['\r', '\n'])));
        }
        while self.partial.len() > MAX_PARTIAL_JOB_LINE_BYTES {
            let end = bounded_char_end(&self.partial, MAX_PARTIAL_JOB_LINE_BYTES);
            let line = self.partial.drain(..end).collect::<String>();
            lines.push(format!("[{job}] {line}\n"));
        }
        if self.partial.is_empty() {
            self.active_job = None;
        }
        lines
    }

    fn finish(&mut self) -> Vec<String> {
        let Some(job) = self.active_job.take() else {
            return Vec::new();
        };
        let line = std::mem::take(&mut self.partial);
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            Vec::new()
        } else {
            vec![format!("[{job}] {line}\n")]
        }
    }
}

fn job_line_end(text: &str) -> Option<usize> {
    let newline = text.find('\n').map(|index| index + 1);
    let carriage_return = text.find('\r').and_then(|index| {
        let following = text.as_bytes().get(index + 1)?;
        Some(index + if *following == b'\n' { 2 } else { 1 })
    });
    match (newline, carriage_return) {
        (Some(newline), Some(carriage_return)) => Some(newline.min(carriage_return)),
        (Some(newline), None) => Some(newline),
        (None, Some(carriage_return)) => Some(carriage_return),
        (None, None) => None,
    }
}

fn bounded_char_end(text: &str, limit: usize) -> usize {
    let mut end = limit.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn print_job_lines(lines: Vec<String>) {
    for line in lines {
        print!("{line}");
    }
}

fn flush_partial_lines_on_stream_error(
    result: anyhow::Result<()>,
    line_buffers: &mut JobLineBuffers,
) -> anyhow::Result<()> {
    if let Err(error) = result {
        print_job_lines(line_buffers.finish());
        return Err(error);
    }
    Ok(())
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

fn run_summary_client() -> anyhow::Result<Client> {
    http_client_builder()
        .timeout(Duration::from_secs(3))
        .build()
        .context("build run summary HTTP client")
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

fn runner_selection_label(selection: &RunRunnerSelection) -> &str {
    match selection {
        RunRunnerSelection::Any => "any runner",
        RunRunnerSelection::Named { name } => name,
        RunRunnerSelection::Mixed => "multiple runners",
    }
}

fn job_state_label(state: RunJobState) -> &'static str {
    match state {
        RunJobState::Blocked => "blocked",
        RunJobState::Queued => "queued",
        RunJobState::Leased => "leased",
        RunJobState::Running => "running",
        RunJobState::Succeeded => "succeeded",
        RunJobState::Failed => "failed",
        RunJobState::Skipped => "skipped",
        RunJobState::Canceled => "canceled",
        RunJobState::Lost => "lost",
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
        assert_eq!(
            runner_selection_label(&RunRunnerSelection::Named {
                name: "linux-one".to_string()
            }),
            "linux-one"
        );
        assert_eq!(
            runner_selection_label(&RunRunnerSelection::Mixed),
            "multiple runners"
        );
        assert_eq!(short_oid("1234567890"), "1234567");
        assert_eq!(short_oid("short"), "short");
    }

    #[test]
    fn run_watch_ignores_replayed_log_positions() {
        let mut cursor = 7;
        assert!(!advance_log_cursor(&mut cursor, 6));
        assert!(!advance_log_cursor(&mut cursor, 7));
        assert!(advance_log_cursor(&mut cursor, 8));
        assert_eq!(cursor, 8);
    }

    #[test]
    fn run_watch_preserves_job_order_when_partial_lines_interleave() {
        let mut buffers = JobLineBuffers::default();
        assert!(buffers.push("backend", "compiling").is_empty());
        assert_eq!(
            buffers.push("web", "testing\n"),
            ["[backend] compiling\n", "[web] testing\n"],
        );
        assert_eq!(
            buffers.push("backend", " complete\nnext"),
            ["[backend]  complete\n"],
        );
        assert_eq!(buffers.finish(), ["[backend] next\n"]);
    }

    #[test]
    fn run_watch_flushes_carriage_returns_and_bounds_partial_lines() {
        let mut buffers = JobLineBuffers::default();
        assert_eq!(
            buffers.push("web", "building 10%\rbuilding 20%\r"),
            ["[web] building 10%\n"],
        );
        assert_eq!(
            buffers.push("web", "done\n"),
            ["[web] building 20%\n", "[web] done\n",]
        );

        let long_line = "x".repeat(MAX_PARTIAL_JOB_LINE_BYTES + 1);
        let flushed = buffers.push("web", &long_line);
        assert_eq!(flushed.len(), 1);
        assert_eq!(
            flushed[0].len(),
            MAX_PARTIAL_JOB_LINE_BYTES + "[web] \n".len()
        );
        assert_eq!(buffers.finish(), ["[web] x\n"]);
    }

    #[test]
    fn run_watch_preserves_crlf_split_across_chunks() {
        let mut buffers = JobLineBuffers::default();
        assert!(buffers.push("windows", "complete\r").is_empty());
        assert_eq!(buffers.push("windows", "\n"), ["[windows] complete\n"]);
    }

    #[test]
    fn run_watch_flushes_partial_lines_before_returning_a_stream_error() {
        let mut buffers = JobLineBuffers::default();
        assert!(buffers.push("backend", "partial diagnostic").is_empty());

        let error = flush_partial_lines_on_stream_error(
            Err(anyhow::anyhow!("stream failed")),
            &mut buffers,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "stream failed");
        assert!(buffers.finish().is_empty());
    }

    #[test]
    fn run_job_labels_cover_scheduler_states() {
        assert_eq!(job_state_label(RunJobState::Blocked), "blocked");
        assert_eq!(job_state_label(RunJobState::Skipped), "skipped");
    }
}
