use super::runs_tests::{
    enqueue, parallel_revision, postgres_store, register_runner, register_runner_with_capacity,
    revision, run, run_for_revision,
    workflow_fixtures::{revision_with_jobs, revision_with_resources},
};
use crate::error::PostgresErrorKind;
use scope_domain::runs::{
    resources::JobResources,
    run::{
        AttemptConclusion, PinnedContainerImage, RunJobState, RunLogChunk, RunState, RunTrigger,
        StepConclusion, StepState,
    },
    workflow::RunnerSelector,
};

#[tokio::test]
async fn atomic_claim_next_skips_jobs_that_do_not_fit_reported_capacity() {
    let store = postgres_store();
    register_runner_with_capacity(&store, "runner-1", "linux-one", 2).await;
    let small = JobResources::new(1_000, 1024 * 1024 * 1024).unwrap();
    let large = JobResources::new(4_000, 4 * 1024 * 1024 * 1024).unwrap();
    let revision = revision_with_resources(&[("large", large), ("small", small)]);
    enqueue(
        &store,
        run_for_revision(
            "run-resource-fit",
            "manual:resource-fit",
            &revision,
            RunnerSelector::Any,
            RunTrigger::Manual,
            Some("user_owner".into()),
        ),
        revision,
    )
    .await;

    let first = store
        .runs()
        .claim_next_job("runner-1", small, "attempt-small", &"a".repeat(64), 20, 80)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.job.key.as_str(), "small");
    assert!(
        store
            .runs()
            .claim_next_job("runner-1", small, "attempt-none", &"b".repeat(64), 21, 81,)
            .await
            .unwrap()
            .is_none()
    );
    let second = store
        .runs()
        .claim_next_job("runner-1", large, "attempt-large", &"c".repeat(64), 22, 82)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.job.key.as_str(), "large");
}
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
            &"a".repeat(64),
            0,
            StepConclusion::Succeeded,
            false,
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
async fn advisory_poll_does_not_wait_for_claim_write_locks() {
    let store = Arc::new(postgres_store());
    register_runner(&store, "runner-1", "linux-box").await;
    enqueue(
        &store,
        run("run-poll-locks", "manual:poll-locks"),
        revision(),
    )
    .await;

    let locks = store.db.begin().await.unwrap();
    for statement in [
        "SELECT 1 FROM scope_run_jobs
         WHERE run_id = 'run-poll-locks' AND job_key = 'checks'
         FOR UPDATE",
        "SELECT 1 FROM scope_runs WHERE id = 'run-poll-locks' FOR UPDATE",
        "SELECT 1 FROM scope_runner_grants WHERE runner_id = 'runner-1' FOR UPDATE",
    ] {
        locks.execute_unprepared(statement).await.unwrap();
    }

    let polling_store = Arc::clone(&store);
    let poll = tokio::spawn(async move {
        polling_store
            .runs()
            .next_dispatchable_job("runner-1", 20)
            .await
    });
    let result = timeout(Duration::from_secs(2), poll).await;
    locks.rollback().await.unwrap();
    let offer = result
        .expect("advisory poll waited for claim write locks")
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(offer.run.id, "run-poll-locks");
    assert_eq!(offer.job.key.as_str(), "checks");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stale_heartbeat_cannot_renew_after_capacity_admits_a_replacement() {
    let store = Arc::new(postgres_store());
    register_runner_with_capacity(&store, "runner-1", "linux-box", 1).await;
    let revision = revision_with_jobs(&["build", "lint"]);
    enqueue(
        &store,
        run_for_revision(
            "run-heartbeat-capacity",
            "manual:heartbeat-capacity",
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
            "run-heartbeat-capacity",
            "build",
            "runner-1",
            "attempt-expiring",
            &"a".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();

    let grant_lock = store.db.begin().await.unwrap();
    grant_lock
        .execute_unprepared(
            "SELECT 1 FROM scope_runner_grants
             WHERE runner_id = 'runner-1'
             FOR UPDATE",
        )
        .await
        .unwrap();
    let grant_blocker_pid = grant_lock
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT pg_backend_pid() AS pid".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i32>("", "pid")
        .unwrap();

    let claim_store = Arc::clone(&store);
    let claim = tokio::spawn(async move {
        claim_store
            .runs()
            .claim_job(
                "run-heartbeat-capacity",
                "lint",
                "runner-1",
                "attempt-replacement",
                &"b".repeat(64),
                80,
                140,
            )
            .await
    });
    let mut claim_holds_runner_lock = false;
    for _ in 0..100 {
        let probe = store.db.begin().await.unwrap();
        let result = probe
            .execute_unprepared(
                "SELECT 1 FROM scope_runners
                 WHERE id = 'runner-1'
                 FOR UPDATE NOWAIT",
            )
            .await;
        let _ = probe.rollback().await;
        if result.is_err() {
            claim_holds_runner_lock = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        claim_holds_runner_lock,
        "claim never reached its runner lock"
    );

    let heartbeat_store = Arc::clone(&store);
    let heartbeat = tokio::spawn(async move {
        heartbeat_store
            .runs()
            .heartbeat_attempt("attempt-expiring", "runner-1", &"a".repeat(64), 79, 139)
            .await
    });
    let mut heartbeat_waits_behind_claim = false;
    for _ in 0..100 {
        heartbeat_waits_behind_claim = store
            .db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT EXISTS (
                     SELECT 1
                     FROM pg_stat_activity claimant
                     JOIN pg_stat_activity heartbeat
                       ON claimant.pid = ANY(pg_blocking_pids(heartbeat.pid))
                     WHERE $1 = ANY(pg_blocking_pids(claimant.pid))
                 ) AS waiting",
                [grant_blocker_pid.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<bool>("", "waiting")
            .unwrap();
        if heartbeat_waits_behind_claim {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        heartbeat_waits_behind_claim,
        "heartbeat never waited behind the replacement claim"
    );

    grant_lock.commit().await.unwrap();
    timeout(Duration::from_secs(5), claim)
        .await
        .expect("replacement claim did not finish")
        .unwrap()
        .unwrap();
    let heartbeat_error = timeout(Duration::from_secs(5), heartbeat)
        .await
        .expect("stale heartbeat did not finish")
        .unwrap()
        .unwrap_err();
    assert_eq!(heartbeat_error.kind, PostgresErrorKind::Unauthenticated);
    assert!(heartbeat_error.message.contains("expired"));
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
async fn log_reads_are_coherent_when_terminal_retention_commits_between_component_lookups() {
    let store = Arc::new(postgres_store());
    register_runner(&store, "runner-logs", "linux-box").await;
    enqueue(
        &store,
        run("run-log-retention", "manual:log-retention"),
        revision(),
    )
    .await;
    store
        .runs()
        .claim_job(
            "run-log-retention",
            "checks",
            "runner-logs",
            "attempt-log-retention",
            &"a".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();
    store
        .runs()
        .pin_attempt_container_image(
            "attempt-log-retention",
            "runner-logs",
            &"a".repeat(64),
            PinnedContainerImage::parse(format!("alpine@sha256:{}", "c".repeat(64))).unwrap(),
            21,
        )
        .await
        .unwrap();
    store
        .runs()
        .start_attempt_step(
            "attempt-log-retention",
            "runner-logs",
            &"a".repeat(64),
            0,
            22,
        )
        .await
        .unwrap();
    store
        .runs()
        .append_attempt_log(
            RunLogChunk::new("attempt-log-retention", 0, 1, "retained\n", 23).unwrap(),
            &"a".repeat(64),
            23,
        )
        .await
        .unwrap();
    store
        .runs()
        .complete_attempt_step(
            "attempt-log-retention",
            &"a".repeat(64),
            0,
            StepConclusion::Succeeded,
            false,
            30,
        )
        .await
        .unwrap();

    let retention = store.db.begin().await.unwrap();
    retention
        .execute_unprepared(
            "DELETE FROM scope_run_logs WHERE run_id = 'run-log-retention';
             LOCK TABLE scope_run_attempts IN ACCESS EXCLUSIVE MODE;
             DELETE FROM scope_run_attempt_steps
              WHERE attempt_id = 'attempt-log-retention';
             DELETE FROM scope_run_attempts
              WHERE id = 'attempt-log-retention';",
        )
        .await
        .unwrap();
    let retention_pid = retention
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT pg_backend_pid() AS pid".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i32>("", "pid")
        .unwrap();
    let page_store = Arc::clone(&store);
    let page = tokio::spawn(async move {
        page_store
            .runs()
            .run_logs_after("run-log-retention", 0, 10)
            .await
    });
    let recent_store = Arc::clone(&store);
    let recent = tokio::spawn(async move {
        recent_store
            .runs()
            .recent_run_logs("run-log-retention", 10)
            .await
    });
    let mut blocked_readers = 0;
    for _ in 0..100 {
        blocked_readers = store
            .db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT count(*) AS count
                   FROM pg_stat_activity activity
                  WHERE activity.pid <> $1
                    AND $1 = ANY(pg_blocking_pids(activity.pid))
                    AND activity.query LIKE '%scope_run_attempts%'",
                [retention_pid.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "count")
            .unwrap();
        if blocked_readers == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        blocked_readers, 2,
        "both log reads must reach their attempt lookup before retention commits"
    );

    retention.commit().await.unwrap();

    let page = page.await.unwrap().unwrap();
    let recent = recent.await.unwrap().unwrap();
    assert!(page.is_empty());
    assert!(recent.logs.is_empty());
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
            &"a".repeat(64),
            AttemptConclusion::SetupFailed {
                exit_code: 1,
                message: "setup failed".to_string(),
            },
            false,
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
