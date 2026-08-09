use super::{enqueue, pin_attempt, postgres_store, register_runner, revision, run};
use crate::{
    db::{entities, generated_ids::test_generated_id},
    error::PostgresErrorKind,
};
use scope_domain::runs::run::{AttemptState, RunState, StepConclusion};
use sea_orm::EntityTrait;

#[tokio::test]
async fn terminal_run_retention_deletes_metadata_and_queues_its_source_atomically() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-box").await;
    let revision = revision();
    let revision_digest = revision.digest().to_string();
    enqueue(&store, run("run-1", "manual:retention"), revision).await;
    store
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
    pin_attempt(&store, "attempt-1", "runner-1", &"a".repeat(64), 21).await;
    store
        .runs()
        .start_attempt_step("attempt-1", "runner-1", &"a".repeat(64), 0, 22)
        .await
        .unwrap();
    store
        .runs()
        .complete_attempt_step(
            "attempt-1",
            &"a".repeat(64),
            0,
            StepConclusion::Succeeded,
            false,
            30,
        )
        .await
        .unwrap();
    let replayed = store
        .runs()
        .complete_attempt_step(
            "attempt-1",
            &"a".repeat(64),
            0,
            StepConclusion::Succeeded,
            false,
            31,
        )
        .await
        .unwrap();
    assert_eq!(replayed.attempt.state, AttemptState::Succeeded);
    assert_eq!(replayed.run.state, RunState::Succeeded);
    assert_eq!(
        store
            .runs()
            .complete_attempt_step(
                "attempt-1",
                &"b".repeat(64),
                0,
                StepConclusion::Succeeded,
                false,
                32,
            )
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Unauthenticated
    );
    store
        .runs()
        .revoke_runner_grant("owner/repo", "runner-1", 33)
        .await
        .unwrap();
    assert_eq!(
        store
            .runs()
            .complete_attempt_step(
                "attempt-1",
                &"a".repeat(64),
                0,
                StepConclusion::Succeeded,
                false,
                34,
            )
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::PermissionDenied
    );

    assert_eq!(
        store
            .runs()
            .prune_terminal_runs(29, 40, 10, &test_generated_id)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .runs()
            .prune_terminal_runs(30, 40, 10, &test_generated_id)
            .await
            .unwrap(),
        1
    );
    assert!(store.runs().run("run-1").await.unwrap().is_none());
    assert!(
        entities::workflow_revision::Entity::find_by_id(revision_digest)
            .one(store.db.as_ref())
            .await
            .unwrap()
            .is_none()
    );
    let cleanup = store
        .cleanup()
        .source_blob_cleanup_batch(400, &test_generated_id)
        .await
        .unwrap();
    assert_eq!(cleanup.pending.len(), 1);
}
