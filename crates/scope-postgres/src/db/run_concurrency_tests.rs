use super::runs_tests::{enqueue, postgres_store, register_runner, revision, run};
use crate::error::PostgresErrorKind;
use scope_domain::runs::run::{AttemptConclusion, RunState, StepState};
use std::sync::Arc;
use tokio::{sync::Barrier, task::JoinSet};

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
