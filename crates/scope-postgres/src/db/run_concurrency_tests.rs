use super::runs_tests::{
    enqueue, parallel_revision, postgres_store, register_runner, revision, run, run_for_revision,
};
use crate::error::PostgresErrorKind;
use scope_domain::runs::run::{
    AttemptConclusion, PinnedContainerImage, RunJobState, RunState, RunTrigger, StepConclusion,
    StepState,
};
use scope_domain::runs::workflow::RunnerSelector;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};
use std::{sync::Arc, time::Duration};
use tokio::{sync::Barrier, task::JoinSet, time::timeout};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_claims_create_exactly_one_active_attempt() {
    let store = Arc::new(postgres_store());
    register_runner(&store, "runner-1", "linux-one").await;
    register_runner(&store, "runner-2", "linux-two").await;
    enqueue(&store, run("run-1", "manual:one"), revision()).await;

    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = JoinSet::new();
    for index in 1..=2 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        tasks.spawn(async move {
            barrier.wait().await;
            store
                .runs()
                .claim_job(
                    "run-1",
                    "checks",
                    &format!("runner-{index}"),
                    &format!("attempt-{index}"),
                    &format!("{index:064x}"),
                    20,
                    80,
                )
                .await
        });
    }

    let mut claims = Vec::new();
    let mut conflicts = 0;
    while let Some(result) = tasks.join_next().await {
        match result.unwrap() {
            Ok(claim) => claims.push(claim),
            Err(error) if error.kind == PostgresErrorKind::Conflict => conflicts += 1,
            Err(error) => panic!("unexpected claim failure: {}", error.message),
        }
    }

    assert_eq!(claims.len(), 1);
    assert_eq!(conflicts, 1);
    let stored = store.runs().run("run-1").await.unwrap().unwrap();
    assert_eq!(stored.state, RunState::Leased);
    assert_eq!(
        claims[0].job.current_attempt_id.as_deref(),
        Some(claims[0].attempt.id.as_str())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn completed_sibling_cannot_regress_running_run_while_another_job_is_leased() {
    let store = Arc::new(postgres_store());
    register_runner(&store, "runner-1", "linux-one").await;
    register_runner(&store, "runner-2", "linux-two").await;
    let revision = parallel_revision();
    enqueue(
        &store,
        run_for_revision(
            "run-state-order",
            "manual:state-order",
            &revision,
            RunnerSelector::Any,
            RunTrigger::Manual,
            Some("user_owner".into()),
        ),
        revision,
    )
    .await;
    let build = store
        .runs()
        .claim_job(
            "run-state-order",
            "build",
            "runner-1",
            "attempt-build",
            &"a".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();
    store
        .runs()
        .claim_job(
            "run-state-order",
            "lint",
            "runner-2",
            "attempt-lint",
            &"b".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();
    store
        .runs()
        .pin_attempt_container_image(
            &build.attempt.id,
            "runner-1",
            &"a".repeat(64),
            PinnedContainerImage::parse(format!("alpine@sha256:{}", "c".repeat(64))).unwrap(),
            21,
        )
        .await
        .unwrap();
    store
        .runs()
        .start_attempt_step(&build.attempt.id, "runner-1", &"a".repeat(64), 0, 22)
        .await
        .unwrap();
    let completed = store
        .runs()
        .complete_attempt_step(
            &build.attempt.id,
            "runner-1",
            &"a".repeat(64),
            0,
            StepConclusion::Succeeded,
            23,
        )
        .await
        .unwrap();

    assert_eq!(completed.run.state, RunState::Running);
    assert_eq!(completed.job.state, RunJobState::Succeeded);
    let detail = store
        .runs()
        .run_detail("run-state-order")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.run.state, RunState::Running);
    assert!(
        detail
            .jobs
            .iter()
            .any(|job| job.state == RunJobState::Leased)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn active_step_cannot_erase_cancellation_committed_after_its_initial_read() {
    let store = Arc::new(postgres_store());
    register_runner(&store, "runner-1", "linux-box").await;
    enqueue(
        &store,
        run("run-cancel-race", "manual:cancel-race"),
        revision(),
    )
    .await;
    store
        .runs()
        .claim_job(
            "run-cancel-race",
            "checks",
            "runner-1",
            "attempt-cancel-race",
            &"a".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();
    store
        .runs()
        .pin_attempt_container_image(
            "attempt-cancel-race",
            "runner-1",
            &"a".repeat(64),
            PinnedContainerImage::parse(format!("alpine@sha256:{}", "b".repeat(64))).unwrap(),
            21,
        )
        .await
        .unwrap();

    let cancellation = store.db.begin().await.unwrap();
    cancellation
        .execute_unprepared(
            "UPDATE scope_runs
             SET cancellation_requested = TRUE, updated_at_unix = 30
             WHERE id = 'run-cancel-race'",
        )
        .await
        .unwrap();

    let active_store = Arc::clone(&store);
    let active = tokio::spawn(async move {
        active_store
            .runs()
            .start_attempt_step("attempt-cancel-race", "runner-1", &"a".repeat(64), 0, 35)
            .await
    });

    let mut active_holds_job_lock = false;
    for _ in 0..100 {
        let probe = store.db.begin().await.unwrap();
        let result = probe
            .execute_unprepared(
                "SELECT 1 FROM scope_run_jobs
                 WHERE run_id = 'run-cancel-race' AND job_key = 'checks'
                 FOR UPDATE NOWAIT",
            )
            .await;
        let _ = probe.rollback().await;
        if result.is_err() {
            active_holds_job_lock = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        active_holds_job_lock,
        "active step never reached its job lock"
    );

    cancellation.commit().await.unwrap();
    let error = timeout(Duration::from_secs(5), active)
        .await
        .expect("active step did not resume after cancellation committed")
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind, PostgresErrorKind::Conflict);

    let detail = store
        .runs()
        .run_detail("run-cancel-race")
        .await
        .unwrap()
        .unwrap();
    assert!(detail.run.cancellation_requested);
    assert_eq!(detail.run.updated_at_unix, 30);
    assert_eq!(detail.jobs[0].state, RunJobState::Leased);
    assert_eq!(detail.attempts[0].steps[0].state, StepState::Pending);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn heartbeat_waiting_for_job_lock_observes_cancellation_committed_during_the_wait() {
    let store = Arc::new(postgres_store());
    register_runner(&store, "runner-1", "linux-box").await;
    enqueue(
        &store,
        run("run-heartbeat-race", "manual:heartbeat-race"),
        revision(),
    )
    .await;
    store
        .runs()
        .claim_job(
            "run-heartbeat-race",
            "checks",
            "runner-1",
            "attempt-heartbeat-race",
            &"a".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();

    let job_lock = store.db.begin().await.unwrap();
    job_lock
        .execute_unprepared(
            "SELECT 1 FROM scope_run_jobs
             WHERE run_id = 'run-heartbeat-race' AND job_key = 'checks'
             FOR UPDATE",
        )
        .await
        .unwrap();
    let blocker_pid = job_lock
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT pg_backend_pid() AS pid".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i32>("", "pid")
        .unwrap();

    let heartbeat_store = Arc::clone(&store);
    let heartbeat = tokio::spawn(async move {
        heartbeat_store
            .runs()
            .heartbeat_attempt(
                "attempt-heartbeat-race",
                "runner-1",
                &"a".repeat(64),
                30,
                100,
            )
            .await
    });
    let mut heartbeat_is_waiting = false;
    for _ in 0..100 {
        heartbeat_is_waiting = store
            .db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT EXISTS (
                     SELECT 1
                     FROM pg_stat_activity activity
                     WHERE activity.pid <> $1
                       AND $1 = ANY(pg_blocking_pids(activity.pid))
                 ) AS waiting",
                [blocker_pid.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<bool>("", "waiting")
            .unwrap();
        if heartbeat_is_waiting {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        heartbeat_is_waiting,
        "heartbeat never waited for the job lock"
    );

    store
        .db
        .execute_unprepared(
            "UPDATE scope_runs
             SET cancellation_requested = TRUE, updated_at_unix = 25
             WHERE id = 'run-heartbeat-race'",
        )
        .await
        .unwrap();
    job_lock.commit().await.unwrap();

    assert!(
        timeout(Duration::from_secs(5), heartbeat)
            .await
            .expect("heartbeat did not resume after the job lock was released")
            .unwrap()
            .unwrap()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn claim_reloads_parent_before_aggregate_save_and_cannot_regress_concurrent_state() {
    let store = Arc::new(postgres_store());
    register_runner(&store, "runner-1", "linux-box").await;
    enqueue(
        &store,
        run("run-claim-race", "manual:claim-race"),
        revision(),
    )
    .await;

    let parent_update = store.db.begin().await.unwrap();
    parent_update
        .execute_unprepared(
            "UPDATE scope_runs
             SET cancellation_requested = TRUE, updated_at_unix = 30
             WHERE id = 'run-claim-race'",
        )
        .await
        .unwrap();

    let claiming_store = Arc::clone(&store);
    let claim = tokio::spawn(async move {
        claiming_store
            .runs()
            .claim_job(
                "run-claim-race",
                "checks",
                "runner-1",
                "attempt-claim-race",
                &"a".repeat(64),
                20,
                80,
            )
            .await
    });

    let mut claim_holds_job_lock = false;
    for _ in 0..100 {
        let probe = store.db.begin().await.unwrap();
        let result = probe
            .execute_unprepared(
                "SELECT 1 FROM scope_run_jobs
                 WHERE run_id = 'run-claim-race' AND job_key = 'checks'
                 FOR UPDATE NOWAIT",
            )
            .await;
        let _ = probe.rollback().await;
        if result.is_err() {
            claim_holds_job_lock = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(claim_holds_job_lock, "claim never reached its job lock");

    parent_update.commit().await.unwrap();
    let error = timeout(Duration::from_secs(5), claim)
        .await
        .expect("claim did not resume after parent update committed")
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind, PostgresErrorKind::Conflict);

    let detail = store
        .runs()
        .run_detail("run-claim-race")
        .await
        .unwrap()
        .unwrap();
    assert!(detail.run.cancellation_requested);
    assert_eq!(detail.run.updated_at_unix, 30);
    assert_eq!(detail.jobs[0].state, RunJobState::Queued);
    assert!(detail.attempts.is_empty());
}

#[tokio::test]
async fn attempt_details_are_newest_first_by_internal_ordinal_with_isolated_steps() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-box").await;
    enqueue(&store, run("run-1", "manual:ordering"), revision()).await;

    store
        .runs()
        .claim_job(
            "run-1",
            "checks",
            "runner-1",
            "attempt-z",
            &"a".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();
    store
        .runs()
        .complete_attempt(
            "attempt-z",
            "runner-1",
            &"a".repeat(64),
            AttemptConclusion::SetupFailed {
                exit_code: 1,
                message: "setup failed".to_string(),
            },
            20,
        )
        .await
        .unwrap();
    store.runs().retry_run("run-1", 20).await.unwrap();
    store
        .runs()
        .claim_job(
            "run-1",
            "checks",
            "runner-1",
            "attempt-a",
            &"b".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();

    let details = store.runs().run_attempt_details("run-1").await.unwrap();
    assert_eq!(
        details
            .iter()
            .map(|detail| detail.attempt.id.as_str())
            .collect::<Vec<_>>(),
        vec!["attempt-a", "attempt-z"]
    );
    assert_eq!(details[0].attempt.runner_name, "linux-box");
    assert_eq!(details[0].steps[0].state, StepState::Pending);
    assert_eq!(details[1].steps[0].state, StepState::Skipped);
}
