use crate::error::ApiError;
use scope_domain::{
    content_ref::ContentRef,
    runs::{
        run::{AttemptConclusion, Run, RunLogChunk, RunSource, RunTrigger, StepConclusion},
        workflow::{
            CompiledWorkflow, ContainerSpec, WorkflowIdentity, WorkflowJob, WorkflowJobId,
            WorkflowPath, WorkflowRevision, WorkflowStep, WorkflowTriggers,
        },
    },
    store::{DEFAULT_GIT_FILE_MODE, SourceBlob, repo_id},
};
use scope_postgres::db::{MetadataStore, RunStore};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const RUNTIME_VERSION: &str = "scope-dev-seed/1";
const DEFAULT_LEASE_SECONDS: u64 = 6 * 60 * 60;

/// Offsets (seconds before "now") for filler runs on the single-job "lint" workflow. These
/// cover short relative times (minutes, hours) plus a spread of days so the runs list exercises
/// every relative-time bucket and pagination past the first page.
const FILLER_LINT_OFFSETS_SECONDS: [u64; 12] = [
    60 * 12,
    60 * 90,
    3600 * 6,
    3600 * 14,
    86_400,
    86_400 * 3,
    86_400 * 6,
    86_400 * 9,
    86_400 * 14,
    86_400 * 21,
    86_400 * 33,
    86_400 * 48,
];

/// Offsets for filler runs on the multi-job "checks" workflow, spread further into the past so
/// the workflow filter and pagination both have plenty of history to page through.
const FILLER_CHECKS_OFFSETS_SECONDS: [u64; 8] = [
    3600 * 4,
    86_400 * 2,
    86_400 * 5,
    86_400 * 11,
    86_400 * 17,
    86_400 * 26,
    86_400 * 40,
    86_400 * 65,
];

/// Seeds a gallery of runs against the `<owner>/public-demo` repository so the runs list and run
/// detail pages can be exercised in a browser. Local-dev only: every state, both workflows, both
/// trigger kinds, an attempt retry, a timed-out attempt, and enough history to page through.
pub(crate) async fn seed_run_gallery(
    metadata: &MetadataStore,
    owner_handle: &str,
    now_unix: u64,
) -> Result<(), ApiError> {
    let repo_id = repo_id(owner_handle, "public-demo");
    let runs = metadata.runs();
    let checks = checks_workflow_revision(&repo_id)?;
    let lint = lint_workflow_revision(&repo_id)?;

    // Run history is ordered by creation sequence, so the gallery has to be
    // written oldest first for the list to read chronologically.
    let mut planned = vec![
        (20, SeededRun::Running),
        (7 * 60, SeededRun::FailedChain),
        (45 * 60, SeededRun::SucceededChain),
        (2 * 3600, SeededRun::Canceled),
        (5 * 3600, SeededRun::RetriedLint),
        (30 * 3600, SeededRun::TimedOut),
    ];
    planned.extend(
        FILLER_LINT_OFFSETS_SECONDS
            .into_iter()
            .enumerate()
            .map(|(index, seconds_ago)| (seconds_ago, SeededRun::FillerLint(index))),
    );
    planned.extend(
        FILLER_CHECKS_OFFSETS_SECONDS
            .into_iter()
            .enumerate()
            .map(|(index, seconds_ago)| (seconds_ago, SeededRun::FillerChecks(index))),
    );
    planned.sort_by(|(left, _), (right, _)| right.cmp(left));

    for (seconds_ago, seeded) in planned {
        let created_at_unix = now_unix.saturating_sub(seconds_ago);
        match seeded {
            SeededRun::Running => seed_running_run(&runs, &checks, created_at_unix).await?,
            SeededRun::FailedChain => {
                seed_failed_chain_run(&runs, &checks, created_at_unix).await?
            }
            SeededRun::SucceededChain => {
                seed_succeeded_chain_run(&runs, &checks, created_at_unix).await?
            }
            SeededRun::Canceled => seed_canceled_run(&runs, &checks, created_at_unix).await?,
            SeededRun::RetriedLint => seed_retried_lint_run(&runs, &lint, created_at_unix).await?,
            SeededRun::TimedOut => seed_timed_out_run(&runs, &lint, created_at_unix).await?,
            SeededRun::FillerLint(index) => {
                seed_filler_lint_run(&runs, &lint, index, created_at_unix).await?
            }
            SeededRun::FillerChecks(index) => {
                seed_filler_checks_run(&runs, &checks, index, created_at_unix).await?
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum SeededRun {
    Canceled,
    FailedChain,
    FillerChecks(usize),
    FillerLint(usize),
    Running,
    RetriedLint,
    SucceededChain,
    TimedOut,
}

// ---------------------------------------------------------------------------------------------
// Workflow definitions
// ---------------------------------------------------------------------------------------------

fn checks_workflow_revision(repo_id: &str) -> Result<WorkflowRevision, ApiError> {
    let container = seed_container()?;
    let build = WorkflowJob::new(
        job_id("build")?,
        vec![],
        container.clone(),
        600,
        vec![],
        BTreeMap::new(),
        vec![step("Build", "cargo build --workspace")?],
    )
    .map_err(ApiError::internal)?;
    let test = WorkflowJob::new(
        job_id("test")?,
        vec![job_id("build")?],
        container.clone(),
        600,
        vec![],
        BTreeMap::new(),
        vec![step("Test", "cargo test --workspace")?],
    )
    .map_err(ApiError::internal)?;
    let deploy = WorkflowJob::new(
        job_id("deploy")?,
        vec![job_id("test")?],
        container,
        900,
        vec![],
        BTreeMap::new(),
        vec![
            step("Package", "scripts/package.sh")?,
            step("Push image", "scripts/push-image.sh")?,
            step("Roll out", "scripts/roll-out.sh")?,
        ],
    )
    .map_err(ApiError::internal)?;
    let definition = CompiledWorkflow::new(
        "Checks",
        WorkflowTriggers::new(true, true).map_err(ApiError::internal)?,
        vec![build, test, deploy],
    )
    .map_err(ApiError::internal)?;
    workflow_revision(repo_id, "/.scope/runs/checks.yml", definition)
}

fn lint_workflow_revision(repo_id: &str) -> Result<WorkflowRevision, ApiError> {
    let lint = WorkflowJob::new(
        job_id("lint")?,
        vec![],
        seed_container()?,
        300,
        vec![],
        BTreeMap::new(),
        vec![step("Lint", "scripts/lint.sh")?],
    )
    .map_err(ApiError::internal)?;
    let definition = CompiledWorkflow::new(
        "Lint",
        WorkflowTriggers::new(true, true).map_err(ApiError::internal)?,
        vec![lint],
    )
    .map_err(ApiError::internal)?;
    workflow_revision(repo_id, "/.scope/runs/lint.yml", definition)
}

fn workflow_revision(
    repo_id: &str,
    path: &str,
    definition: CompiledWorkflow,
) -> Result<WorkflowRevision, ApiError> {
    let identity = WorkflowIdentity::new(
        repo_id.to_string(),
        WorkflowPath::parse(path).map_err(ApiError::internal)?,
    )
    .map_err(ApiError::internal)?;
    WorkflowRevision::new(identity, definition).map_err(ApiError::internal)
}

fn seed_container() -> Result<ContainerSpec, ApiError> {
    ContainerSpec::new(format!(
        "ghcr.io/scope/dev-seed-ci@sha256:{}",
        fake_digest("scope-dev-seed-container-image")
    ))
    .map_err(ApiError::internal)
}

fn job_id(id: &str) -> Result<WorkflowJobId, ApiError> {
    WorkflowJobId::parse(id).map_err(ApiError::internal)
}

fn step(name: &str, run: &str) -> Result<WorkflowStep, ApiError> {
    WorkflowStep::new(name, run).map_err(ApiError::internal)
}

// ---------------------------------------------------------------------------------------------
// Named scenarios
// ---------------------------------------------------------------------------------------------

async fn seed_running_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let run_id = enqueue(
        runs,
        revision,
        "running-chain",
        RunTrigger::Manual,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    run_job(
        runs,
        &run_id,
        "build",
        1,
        &[StepPlan::succeed(lines(&[
            "Compiling workspace",
            "Build finished in 12.4s",
        ]))],
        &mut clock,
    )
    .await?;
    run_job(
        runs,
        &run_id,
        "test",
        1,
        &[StepPlan::succeed(lines(&[
            "Running 480 tests",
            "test result: ok. 480 passed; 0 failed",
        ]))],
        &mut clock,
    )
    .await?;
    run_job(
        runs,
        &run_id,
        "deploy",
        1,
        &[
            StepPlan::succeed(lines(&[
                "Packaging release artifact",
                "Package created: build/release.tar.gz",
            ])),
            StepPlan::succeed(lines(&[
                "Pushing image to registry",
                "Pushed ghcr.io/scope/app:sha-abc1234",
            ])),
            StepPlan::running(rollout_log_chunks()),
        ],
        &mut clock,
    )
    .await?;
    Ok(())
}

async fn seed_failed_chain_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let run_id = enqueue(
        runs,
        revision,
        "failed-chain",
        RunTrigger::PushMain,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    run_job(
        runs,
        &run_id,
        "build",
        1,
        &[StepPlan::fail(
            101,
            lines(&[
                "Compiling workspace",
                "error[E0433]: failed to resolve: use of undeclared crate `scope_runtime`",
                "error: could not compile `api` (bin \"api\") due to 1 previous error",
            ]),
        )],
        &mut clock,
    )
    .await?;
    Ok(())
}

async fn seed_succeeded_chain_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let run_id = enqueue(
        runs,
        revision,
        "succeeded-chain",
        RunTrigger::Manual,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    seed_checks_chain_success(runs, &run_id, &mut clock).await
}

async fn seed_canceled_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let run_id = enqueue(
        runs,
        revision,
        "canceled-chain",
        RunTrigger::PushMain,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    run_job(
        runs,
        &run_id,
        "build",
        1,
        &[StepPlan::succeed(lines(&[
            "Compiling workspace",
            "Build finished in 9.8s",
        ]))],
        &mut clock,
    )
    .await?;

    let attempt = attempt_id(&run_id, "test", 1);
    let token = attempt_token(&run_id, "test", 1);
    clock += 1;
    runs.dispatch_job(
        &run_id,
        "test",
        &attempt,
        &token,
        RUNTIME_VERSION,
        clock,
        clock + DEFAULT_LEASE_SECONDS,
    )
    .await?;
    clock += 1;
    runs.start_attempt_step(&attempt, &token, 0, clock).await?;
    clock += 1;
    append_log(
        runs,
        &attempt,
        &token,
        0,
        1,
        lines(&["Running test suite", "112 of 480 tests complete"])
            .into_iter()
            .next()
            .expect("single log chunk"),
        clock,
    )
    .await?;
    clock += 1;
    runs.request_run_cancellation(&run_id, clock).await?;
    clock += 1;
    runs.complete_attempt(&attempt, &token, AttemptConclusion::Canceled, false, clock)
        .await?;
    Ok(())
}

async fn seed_retried_lint_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let run_id = enqueue(
        runs,
        revision,
        "retried-lint",
        RunTrigger::Manual,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    run_job(
        runs,
        &run_id,
        "lint",
        1,
        &[StepPlan::fail(
            1,
            lines(&[
                "Linting changed files",
                "error: unused import `std::fmt::Debug`",
            ]),
        )],
        &mut clock,
    )
    .await?;
    clock += 1;
    runs.retry_run(&run_id, clock).await?;
    run_job(
        runs,
        &run_id,
        "lint",
        2,
        &[StepPlan::succeed(lines(&[
            "Linting changed files",
            "no lint issues found",
        ]))],
        &mut clock,
    )
    .await?;
    Ok(())
}

async fn seed_timed_out_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let run_id = enqueue(
        runs,
        revision,
        "timed-out-lint",
        RunTrigger::PushMain,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    let attempt = attempt_id(&run_id, "lint", 1);
    let token = attempt_token(&run_id, "lint", 1);
    runs.dispatch_job(
        &run_id,
        "lint",
        &attempt,
        &token,
        RUNTIME_VERSION,
        clock,
        clock + DEFAULT_LEASE_SECONDS,
    )
    .await?;
    clock += 1;
    runs.start_attempt_step(&attempt, &token, 0, clock).await?;
    clock += 1;
    append_log(
        runs,
        &attempt,
        &token,
        0,
        1,
        vec!["Linting changed files".to_string()],
        clock,
    )
    .await?;
    clock += 1;
    runs.complete_attempt(&attempt, &token, AttemptConclusion::TimedOut, false, clock)
        .await?;
    Ok(())
}

async fn seed_filler_lint_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    index: usize,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let trigger = if index.is_multiple_of(2) {
        RunTrigger::Manual
    } else {
        RunTrigger::PushMain
    };
    let run_id = enqueue(
        runs,
        revision,
        &format!("filler-lint-{index}"),
        trigger,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    let plan = if index.is_multiple_of(3) {
        StepPlan::fail(
            1,
            lines(&["Linting changed files", "error: missing trailing newline"]),
        )
    } else {
        StepPlan::succeed(lines(&["Linting changed files", "no lint issues found"]))
    };
    run_job(runs, &run_id, "lint", 1, &[plan], &mut clock).await?;
    Ok(())
}

async fn seed_filler_checks_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    index: usize,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let trigger = if index.is_multiple_of(2) {
        RunTrigger::PushMain
    } else {
        RunTrigger::Manual
    };
    let run_id = enqueue(
        runs,
        revision,
        &format!("filler-checks-{index}"),
        trigger,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    seed_checks_chain_success(runs, &run_id, &mut clock).await
}

async fn seed_checks_chain_success(
    runs: &RunStore,
    run_id: &str,
    clock: &mut u64,
) -> Result<(), ApiError> {
    run_job(
        runs,
        run_id,
        "build",
        1,
        &[StepPlan::succeed(lines(&[
            "Compiling workspace",
            "Build finished in 10.1s",
        ]))],
        clock,
    )
    .await?;
    run_job(
        runs,
        run_id,
        "test",
        1,
        &[StepPlan::succeed(lines(&[
            "Running 480 tests",
            "test result: ok. 480 passed; 0 failed",
        ]))],
        clock,
    )
    .await?;
    run_job(
        runs,
        run_id,
        "deploy",
        1,
        &[
            StepPlan::succeed(lines(&["Packaging release artifact", "Package created"])),
            StepPlan::succeed(lines(&["Pushing image to registry", "Pushed successfully"])),
            StepPlan::succeed(lines(&["Rolling out release", "Rollout complete"])),
        ],
        clock,
    )
    .await
}

// ---------------------------------------------------------------------------------------------
// Choreography helpers
// ---------------------------------------------------------------------------------------------

enum StepOutcome {
    Succeed,
    Fail(i32),
}

struct StepPlan {
    log_chunks: Vec<Vec<String>>,
    outcome: Option<StepOutcome>,
}

impl StepPlan {
    fn succeed(log_chunks: Vec<Vec<String>>) -> Self {
        Self {
            log_chunks,
            outcome: Some(StepOutcome::Succeed),
        }
    }

    fn fail(exit_code: i32, log_chunks: Vec<Vec<String>>) -> Self {
        Self {
            log_chunks,
            outcome: Some(StepOutcome::Fail(exit_code)),
        }
    }

    fn running(log_chunks: Vec<Vec<String>>) -> Self {
        Self {
            log_chunks,
            outcome: None,
        }
    }
}

async fn enqueue(
    runs: &RunStore,
    revision: &WorkflowRevision,
    slug: &str,
    trigger: RunTrigger,
    created_at_unix: u64,
) -> Result<String, ApiError> {
    let requested_by_user_id =
        matches!(trigger, RunTrigger::Manual).then(|| super::DEV_SEED_USER_ID.to_string());
    let run = Run::new(
        format!("run_dev_seed_{slug}"),
        format!("dev-seed:{slug}"),
        revision.workflow().clone(),
        revision.digest().to_string(),
        trigger,
        requested_by_user_id,
        fake_run_source(slug)?,
        created_at_unix,
    )
    .map_err(ApiError::internal)?;
    let enqueued = runs.enqueue_run(run, revision.clone()).await?;
    Ok(enqueued.run.id)
}

/// Dispatches an attempt for `job_key` and drives it through the given step plan. A plan whose
/// last step has no outcome leaves the attempt (and therefore the run) running.
async fn run_job(
    runs: &RunStore,
    run_id: &str,
    job_key: &str,
    attempt_number: u32,
    steps: &[StepPlan],
    clock: &mut u64,
) -> Result<(), ApiError> {
    let attempt = attempt_id(run_id, job_key, attempt_number);
    let token = attempt_token(run_id, job_key, attempt_number);
    *clock += 1;
    runs.dispatch_job(
        run_id,
        job_key,
        &attempt,
        &token,
        RUNTIME_VERSION,
        *clock,
        *clock + DEFAULT_LEASE_SECONDS,
    )
    .await?;

    let mut sequence = 1u64;
    for (index, plan) in steps.iter().enumerate() {
        let step_index = u32::try_from(index).map_err(ApiError::internal)?;
        *clock += 1;
        runs.start_attempt_step(&attempt, &token, step_index, *clock)
            .await?;
        for chunk in &plan.log_chunks {
            *clock += 1;
            append_log(
                runs,
                &attempt,
                &token,
                step_index,
                sequence,
                chunk.clone(),
                *clock,
            )
            .await?;
            sequence += 1;
        }
        match plan.outcome {
            Some(StepOutcome::Succeed) => {
                *clock += 1;
                runs.complete_attempt_step(
                    &attempt,
                    &token,
                    step_index,
                    StepConclusion::Succeeded,
                    false,
                    *clock,
                )
                .await?;
            }
            Some(StepOutcome::Fail(exit_code)) => {
                *clock += 1;
                runs.complete_attempt_step(
                    &attempt,
                    &token,
                    step_index,
                    StepConclusion::Failed { exit_code },
                    false,
                    *clock,
                )
                .await?;
                return Ok(());
            }
            None => return Ok(()),
        }
    }
    *clock += 1;
    runs.complete_attempt(
        &attempt,
        &token,
        AttemptConclusion::Succeeded,
        false,
        *clock,
    )
    .await?;
    Ok(())
}

async fn append_log(
    runs: &RunStore,
    attempt_id: &str,
    token: &str,
    step_index: u32,
    sequence: u64,
    log_lines: Vec<String>,
    now_unix: u64,
) -> Result<(), ApiError> {
    let text = format!("{}\n", log_lines.join("\n"));
    let chunk = RunLogChunk::new(attempt_id.to_string(), step_index, sequence, text, now_unix)
        .map_err(ApiError::internal)?;
    runs.append_attempt_log(chunk, token, now_unix).await?;
    Ok(())
}

fn rollout_log_chunks() -> Vec<Vec<String>> {
    (0..5)
        .map(|batch: u32| {
            (0..60u32)
                .map(|offset| {
                    let n = batch * 60 + offset + 1;
                    format!("[roll-out] step {n}/300: reconciling replica set generation {n}")
                })
                .collect()
        })
        .collect()
}

fn lines(items: &[&str]) -> Vec<Vec<String>> {
    vec![items.iter().map(|item| item.to_string()).collect()]
}

fn attempt_id(run_id: &str, job_key: &str, attempt_number: u32) -> String {
    format!("attempt_{run_id}_{job_key}_{attempt_number}")
}

fn attempt_token(run_id: &str, job_key: &str, attempt_number: u32) -> String {
    fake_digest(&format!("token:{run_id}:{job_key}:{attempt_number}"))
}

fn fake_run_source(label: &str) -> Result<RunSource, ApiError> {
    let sha256 = fake_digest(&format!("source:{label}"));
    let object = SourceBlob {
        content_ref: ContentRef::git_bundle_sha256(sha256.clone()),
        sha256,
        git_oid: fake_git_oid(label),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 1,
    };
    RunSource::ephemeral_git_bundle(object).map_err(ApiError::internal)
}

fn fake_digest(label: &str) -> String {
    hex::encode(Sha256::digest(label.as_bytes()))
}

fn fake_git_oid(label: &str) -> String {
    fake_digest(&format!("git-oid:{label}"))[..40].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo_seed::DevSeedUser;
    use scope_domain::runs::run::RunState;
    use scope_object_store::{EncryptedObjectStore, MemoryObjectStore};
    use scope_postgres::db::RunHistoryPageQuery;
    use std::sync::Arc;

    #[tokio::test]
    async fn gallery_covers_every_run_state_and_both_workflows() {
        let object_store = EncryptedObjectStore::new(Arc::new(MemoryObjectStore::new()), [7; 32]);
        let catalog = super::super::catalog(
            &object_store,
            DevSeedUser {
                email: "dev@example.com".to_string(),
                handle: "dev".to_string(),
            },
        )
        .unwrap();
        let target = scope_postgres::db::TestDatabaseTarget::required().unwrap();
        let metadata = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        metadata.admin().seed_catalog_for_tests(catalog).unwrap();

        let now_unix = 1_900_000_000;
        seed_run_gallery(&metadata, "dev", now_unix).await.unwrap();

        let repository_id = repo_id("dev", "public-demo");
        let page = metadata
            .runs()
            .repository_run_history_page(RunHistoryPageQuery {
                repository_id: &repository_id,
                workflow_path: None,
                after: None,
                limit: 100,
            })
            .await
            .unwrap();
        assert!(page.len() >= 25, "expected at least 25 seeded runs");

        let workflow_paths = page
            .iter()
            .map(|entry| entry.run.workflow.path().as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(workflow_paths.len(), 2);

        for expected in [
            RunState::Running,
            RunState::Failed,
            RunState::Succeeded,
            RunState::Canceled,
        ] {
            assert!(
                page.iter().any(|entry| entry.run.state == expected),
                "missing a run in state {expected:?}"
            );
        }

        let retried = page
            .iter()
            .find(|entry| entry.run.id == "run_dev_seed_retried-lint")
            .expect("retried lint run is seeded");
        assert_eq!(retried.run.state, RunState::Succeeded);
        assert_eq!(retried.jobs.len(), 1);
        assert_eq!(retried.jobs[0].last_attempt_number, 2);

        let running = page
            .iter()
            .find(|entry| entry.run.id == "run_dev_seed_running-chain")
            .expect("running chain run is seeded");
        assert_eq!(running.jobs.len(), 3);
    }
}
