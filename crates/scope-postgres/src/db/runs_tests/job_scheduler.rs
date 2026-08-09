use super::{
    enqueue, postgres_store, register_runner, register_runner_with_capacity, revision, run,
    run_for_revision, runner_with_capacity, workflow_fixtures::revision_with_jobs,
};
use crate::error::PostgresErrorKind;
use scope_domain::runs::{
    run::{
        AttemptConclusion, AttemptState, PinnedContainerImage, RunJobState, RunState, RunTrigger,
    },
    workflow::{RunnerSelector, WorkflowRevision},
};
use sea_orm::ConnectionTrait;

#[tokio::test]
async fn runner_capacity_round_trips_through_persistence() {
    let store = postgres_store();
    let runner = runner_with_capacity("runner-capacity", 4);
    store.runs().register_runner(runner.clone()).await.unwrap();

    let persisted = store.runs().runner(&runner.id).await.unwrap().unwrap();
    assert_eq!(persisted.max_concurrent_jobs.get(), 4);
    assert_eq!(persisted, runner);
}

#[tokio::test]
async fn dispatch_query_ignores_unmaterializable_rows_and_selects_named_or_any_jobs() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-one").await;
    register_runner(&store, "runner-2", "linux-two").await;
    enqueue(&store, run("run-00-invalid", "manual:invalid"), revision()).await;
    store
        .db
        .execute_unprepared(
            "UPDATE scope_run_jobs
             SET desired_runner_name = 'invalid runner name'
             WHERE run_id = 'run-00-invalid' AND job_key = 'checks'",
        )
        .await
        .unwrap();

    assert!(
        store
            .runs()
            .next_dispatchable_job("runner-1", 10)
            .await
            .unwrap()
            .is_none()
    );

    enqueue(
        &store,
        run_for_revision(
            "run-01-named",
            "manual:named",
            &revision(),
            RunnerSelector::named("linux-two").unwrap(),
            RunTrigger::Manual,
            Some("user_owner".into()),
        ),
        revision(),
    )
    .await;
    enqueue(&store, run("run-02-any", "manual:any"), revision()).await;

    assert_eq!(
        store
            .runs()
            .next_dispatchable_job("runner-1", 10)
            .await
            .unwrap()
            .unwrap()
            .run
            .id,
        "run-02-any"
    );
    assert_eq!(
        store
            .runs()
            .next_dispatchable_job("runner-2", 10)
            .await
            .unwrap()
            .unwrap()
            .run
            .id,
        "run-01-named"
    );
}

#[tokio::test]
async fn run_snapshot_returns_the_run_jobs_and_log_state_together() {
    let store = postgres_store();
    let revision = parallel_revision();
    enqueue(
        &store,
        run_for_revision(
            "run-snapshot",
            "manual:snapshot",
            &revision,
            RunnerSelector::Any,
            RunTrigger::Manual,
            Some("user_owner".into()),
        ),
        revision,
    )
    .await;

    let snapshot = store
        .runs()
        .run_snapshot("run-snapshot")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.run.id, "run-snapshot");
    assert_eq!(
        snapshot
            .jobs
            .iter()
            .map(|job| job.key.as_str())
            .collect::<Vec<_>>(),
        ["build", "lint"]
    );
    assert!(!snapshot.logs_truncated);
}

#[tokio::test]
async fn runner_capacity_is_authoritative_across_concurrent_job_claims() {
    let store = postgres_store();
    register_runner_with_capacity(&store, "runner-1", "linux-box", 2).await;
    let revision = capacity_revision();
    enqueue(
        &store,
        run_for_revision(
            "run-parallel",
            "manual:parallel",
            &revision,
            RunnerSelector::Any,
            RunTrigger::Manual,
            Some("user_owner".into()),
        ),
        revision,
    )
    .await;

    let build_runs = store.runs();
    let lint_runs = store.runs();
    let build_token = "a".repeat(64);
    let lint_token = "b".repeat(64);
    let (build, lint) = tokio::join!(
        build_runs.claim_job(
            "run-parallel",
            "build",
            "runner-1",
            "attempt-build",
            &build_token,
            20,
            80,
        ),
        lint_runs.claim_job(
            "run-parallel",
            "lint",
            "runner-1",
            "attempt-lint",
            &lint_token,
            20,
            80,
        ),
    );
    let build = build.unwrap();
    let lint = lint.unwrap();
    assert_eq!(build.attempt.number, 1);
    assert_eq!(lint.attempt.number, 1);
    assert_eq!(build.attempt.job_key.as_str(), "build");
    assert_eq!(lint.attempt.job_key.as_str(), "lint");
    assert!(
        store
            .runs()
            .next_dispatchable_job("runner-1", 20)
            .await
            .unwrap()
            .is_none()
    );
    let at_capacity = store
        .runs()
        .claim_job(
            "run-parallel",
            "test",
            "runner-1",
            "attempt-test-blocked",
            &"c".repeat(64),
            20,
            80,
        )
        .await
        .unwrap_err();
    assert_eq!(at_capacity.kind, PostgresErrorKind::ResourceExhausted);

    store
        .runs()
        .complete_attempt(
            &build.attempt.id,
            &build_token,
            AttemptConclusion::SetupFailed {
                exit_code: 1,
                message: "build setup failed".into(),
            },
            false,
            21,
        )
        .await
        .unwrap();
    let test = store
        .runs()
        .claim_job(
            "run-parallel",
            "test",
            "runner-1",
            "attempt-test",
            &"d".repeat(64),
            22,
            82,
        )
        .await
        .unwrap();
    assert_eq!(test.attempt.job_key.as_str(), "test");
    assert_eq!(
        store
            .runs()
            .run("run-parallel")
            .await
            .unwrap()
            .unwrap()
            .state,
        RunState::Leased
    );
}

#[tokio::test]
async fn run_detail_preserves_attempt_history_bounded_per_job() {
    let store = postgres_store();
    register_runner(&store, "runner-history", "linux-box").await;
    let revision = parallel_revision();
    enqueue(
        &store,
        run_for_revision(
            "run-history",
            "manual:history",
            &revision,
            RunnerSelector::Any,
            RunTrigger::Manual,
            Some("user_owner".into()),
        ),
        revision,
    )
    .await;
    store
        .db
        .execute_unprepared(
            "INSERT INTO scope_run_attempts (
                 id, run_id, job_key, number, runner_id, runner_name, token_hash,
                 token_expires_at_unix, state, lease_expires_at_unix,
                 last_heartbeat_at_unix, created_at_unix, started_at_unix,
                 completed_at_unix, terminal_reason, log_bytes,
                 first_truncated_step_index
             )
             SELECT 'attempt-' || job_key || '-' || number,
                    'run-history', job_key, number, 'runner-history', 'linux-box',
                    lpad(to_hex(row_number() OVER ()), 64, '0'),
                    1000, 'failed', 1000, number, number, NULL, number,
                    jsonb_build_object(
                        'kind', 'runner-setup-failed',
                        'exit_code', 1,
                        'message', 'setup failed'
                    ),
                    0, NULL
             FROM unnest(ARRAY['build', 'lint']) AS job_key
             CROSS JOIN generate_series(1, 51) AS number;
             INSERT INTO scope_run_attempt_steps (
                 attempt_id, step_index, state, started_at_unix,
                 completed_at_unix, exit_code
             )
             SELECT id, 0, 'skipped', NULL, completed_at_unix, NULL
             FROM scope_run_attempts
             WHERE run_id = 'run-history'",
        )
        .await
        .unwrap();

    let details = store
        .runs()
        .run_attempt_details("run-history")
        .await
        .unwrap();

    assert_eq!(details.len(), 102);
    for job_key in ["build", "lint"] {
        assert_eq!(
            details
                .iter()
                .filter(|detail| detail.attempt.job_key.as_str() == job_key)
                .count(),
            51
        );
    }
}

#[tokio::test]
async fn expired_attempts_release_capacity_at_the_lease_deadline() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-box").await;
    let revision = capacity_revision();
    enqueue(
        &store,
        run_for_revision(
            "run-expired-capacity",
            "manual:expired-capacity",
            &revision,
            RunnerSelector::Any,
            RunTrigger::Manual,
            Some("user_owner".into()),
        ),
        revision,
    )
    .await;

    store
        .runs()
        .claim_job(
            "run-expired-capacity",
            "build",
            "runner-1",
            "attempt-expired",
            &"a".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();

    assert!(
        store
            .runs()
            .next_dispatchable_job("runner-1", 79)
            .await
            .unwrap()
            .is_none()
    );
    let live_capacity = store
        .runs()
        .claim_job(
            "run-expired-capacity",
            "lint",
            "runner-1",
            "attempt-live-blocked",
            &"b".repeat(64),
            79,
            139,
        )
        .await
        .unwrap_err();
    assert_eq!(live_capacity.kind, PostgresErrorKind::ResourceExhausted);

    let offer = store
        .runs()
        .next_dispatchable_job("runner-1", 80)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(offer.job.key.as_str(), "lint");
    let next = store
        .runs()
        .claim_job(
            "run-expired-capacity",
            "lint",
            "runner-1",
            "attempt-next",
            &"c".repeat(64),
            80,
            140,
        )
        .await
        .unwrap();
    assert_eq!(next.attempt.id, "attempt-next");

    assert_eq!(
        store.runs().expired_attempt_ids(80, 10).await.unwrap(),
        vec!["attempt-expired"]
    );
    let recovered = store
        .runs()
        .expire_attempt("attempt-expired", 80)
        .await
        .unwrap();
    assert_eq!(recovered.attempt.state, AttemptState::Lost);
    assert_eq!(recovered.job.state, RunJobState::Queued);
}

#[tokio::test]
async fn independent_claim_with_older_timestamp_does_not_regress_the_aggregate() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-one").await;
    register_runner(&store, "runner-2", "linux-two").await;
    let revision = parallel_revision();
    enqueue(
        &store,
        run_for_revision(
            "run-out-of-order",
            "manual:out-of-order",
            &revision,
            RunnerSelector::Any,
            RunTrigger::Manual,
            Some("user_owner".into()),
        ),
        revision,
    )
    .await;

    store
        .runs()
        .claim_job(
            "run-out-of-order",
            "build",
            "runner-1",
            "attempt-build-late",
            &"a".repeat(64),
            30,
            90,
        )
        .await
        .unwrap();
    store
        .runs()
        .claim_job(
            "run-out-of-order",
            "lint",
            "runner-2",
            "attempt-lint-early",
            &"b".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();

    let detail = store
        .runs()
        .run_detail("run-out-of-order")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.run.state, RunState::Leased);
    assert_eq!(detail.run.updated_at_unix, 30);
    assert!(
        detail
            .jobs
            .iter()
            .all(|job| job.state == scope_domain::runs::run::RunJobState::Leased)
    );
}

#[tokio::test]
async fn active_cancellation_is_intent_until_runner_acknowledges() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-box").await;
    enqueue(&store, run("run-1", "manual:cancel"), revision()).await;
    let claim = store
        .runs()
        .claim_job(
            "run-1",
            "checks",
            "runner-1",
            "attempt-1",
            &"a".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();

    let canceling = store
        .runs()
        .request_run_cancellation("run-1", 30)
        .await
        .unwrap();
    assert_eq!(canceling.state, RunState::Leased);
    assert!(canceling.cancellation_requested);
    assert_eq!(
        store
            .runs()
            .start_attempt_step(&claim.attempt.id, "runner-1", &"a".repeat(64), 0, 35,)
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Conflict
    );
    assert!(
        store
            .runs()
            .heartbeat_attempt(&claim.attempt.id, "runner-1", &"a".repeat(64), 40, 100,)
            .await
            .unwrap()
    );
    let completed = store
        .runs()
        .complete_attempt(
            &claim.attempt.id,
            &"a".repeat(64),
            AttemptConclusion::Canceled,
            false,
            50,
        )
        .await
        .unwrap();
    assert_eq!(completed.run.state, RunState::Canceled);
    assert!(completed.run.cancellation_requested);
    assert_eq!(completed.attempt.state, AttemptState::Canceled);
    let persisted = store.runs().run("run-1").await.unwrap().unwrap();
    assert_eq!(persisted.state, RunState::Canceled);
    assert!(persisted.cancellation_requested);
    assert_eq!(
        store
            .runs()
            .heartbeat_attempt(&claim.attempt.id, "runner-1", &"a".repeat(64), 60, 120,)
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Conflict
    );
}

#[tokio::test]
async fn retry_persists_the_jobs_pinned_container_image() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-box").await;
    enqueue(&store, run("run-retry-pin", "manual:retry-pin"), revision()).await;
    store
        .runs()
        .claim_job(
            "run-retry-pin",
            "checks",
            "runner-1",
            "attempt-retry-pin",
            &"a".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();
    let image =
        PinnedContainerImage::parse(format!("registry.example/job@sha256:{}", "b".repeat(64)))
            .unwrap();
    store
        .runs()
        .pin_attempt_container_image(
            "attempt-retry-pin",
            "runner-1",
            &"a".repeat(64),
            image.clone(),
            21,
        )
        .await
        .unwrap();
    store
        .runs()
        .complete_attempt(
            "attempt-retry-pin",
            &"a".repeat(64),
            AttemptConclusion::SetupFailed {
                exit_code: 1,
                message: "setup failed".into(),
            },
            false,
            22,
        )
        .await
        .unwrap();

    store.runs().retry_run("run-retry-pin", 23).await.unwrap();

    let detail = store
        .runs()
        .run_detail("run-retry-pin")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.jobs[0].pinned_container_image, Some(image));
}

pub(crate) fn parallel_revision() -> WorkflowRevision {
    revision_with_jobs(&["build", "lint"])
}

fn capacity_revision() -> WorkflowRevision {
    revision_with_jobs(&["build", "lint", "test"])
}
