use crate::db::entities;
use crate::db::{
    MetadataStore, TestDatabaseTarget, cleanup_queue::types::SourceBlobCleanupDecision,
};
use scope_domain::content::{DEFAULT_GIT_FILE_MODE, SourceBlob};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
use std::time::Duration;

#[tokio::test]
async fn content_fence_serializes_publication_with_delete_revalidation() {
    let target = TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    let blob = blob("fenced-publication");
    store
        .cleanup()
        .queue_pending_source_blob_deletions(
            vec![blob.clone()],
            now(),
            &super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
    make_source_blob_cleanup_due(&store, &blob.content_ref).await;
    let batch = store
        .cleanup()
        .source_blob_cleanup_batch(now(), &super::generated_ids::test_generated_id)
        .await
        .unwrap();

    let publication_fence = store
        .acquire_content_ref_fence(std::slice::from_ref(&blob.content_ref))
        .await
        .unwrap();
    let cleanup_store = store.clone();
    let cleanup_ref = blob.content_ref.clone();
    let waiting_cleanup = tokio::spawn(async move {
        cleanup_store
            .acquire_content_ref_fence(std::slice::from_ref(&cleanup_ref))
            .await
            .unwrap()
    });
    let mut waiting_cleanup = Box::pin(waiting_cleanup);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), waiting_cleanup.as_mut())
            .await
            .is_err(),
        "cleanup must wait while publication owns the content fence"
    );

    super::object_references::insert_object_reference(
        store.db.as_ref(),
        "test-publication",
        "fenced-publication",
        &blob,
    )
    .await
    .unwrap();
    publication_fence.release().await;
    let cleanup_fence = waiting_cleanup.await.unwrap();
    assert_eq!(
        store
            .cleanup()
            .source_blob_cleanup_decision(&batch, &blob)
            .await
            .unwrap(),
        SourceBlobCleanupDecision::Referenced
    );
    cleanup_fence.release().await;
}

#[tokio::test]
async fn cleanup_claims_are_bounded_and_failed_work_is_backed_off() {
    let target = TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    let blob = blob("retry-blob");
    store
        .cleanup()
        .queue_pending_source_blob_deletions(
            vec![blob.clone()],
            now(),
            &super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
    make_source_blob_cleanup_due(&store, &blob.content_ref).await;

    let claimed = store
        .cleanup()
        .source_blob_cleanup_batch(now(), &super::generated_ids::test_generated_id)
        .await
        .unwrap();
    assert_eq!(claimed.pending, vec![blob.clone()]);
    assert!(
        store
            .cleanup()
            .source_blob_cleanup_batch(now(), &super::generated_ids::test_generated_id)
            .await
            .unwrap()
            .pending
            .is_empty(),
        "an active claim must hide work from concurrent drains"
    );

    store
        .cleanup()
        .finish_source_blob_cleanup(
            claimed,
            std::slice::from_ref(&blob),
            now(),
            &super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
    let immediate_retry = store
        .cleanup()
        .source_blob_cleanup_batch(now(), &super::generated_ids::test_generated_id)
        .await
        .unwrap();
    assert_eq!(immediate_retry.pending, vec![blob.clone()]);
    store
        .cleanup()
        .finish_source_blob_cleanup(
            immediate_retry,
            std::slice::from_ref(&blob),
            now(),
            &super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
    assert!(
        store
            .cleanup()
            .source_blob_cleanup_batch(now(), &super::generated_ids::test_generated_id)
            .await
            .unwrap()
            .pending
            .is_empty(),
        "failed cleanup must wait for its retry backoff"
    );
}

#[tokio::test]
async fn reclaimed_source_blob_rejects_stale_completion() {
    let target = TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    let blob = blob("reclaimed");
    let content_ref = blob.content_ref.clone();
    store
        .cleanup()
        .queue_pending_source_blob_deletions(
            vec![blob],
            now(),
            &super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
    make_source_blob_cleanup_due(&store, &content_ref).await;
    let stale = store
        .cleanup()
        .source_blob_cleanup_batch(now(), &super::generated_ids::test_generated_id)
        .await
        .unwrap();
    make_source_blob_cleanup_due(&store, &content_ref).await;
    let current = store
        .cleanup()
        .source_blob_cleanup_batch(now(), &super::generated_ids::test_generated_id)
        .await
        .unwrap();

    store
        .cleanup()
        .finish_source_blob_cleanup(stale, &[], now(), &super::generated_ids::test_generated_id)
        .await
        .unwrap();
    let row = entities::source_blob_cleanup_job::Entity::find_by_id(
        serde_json::to_string(&content_ref).unwrap(),
    )
    .one(store.db.as_ref())
    .await
    .unwrap()
    .unwrap();
    assert!(row.completed_at_unix.is_none());
    store
        .cleanup()
        .finish_source_blob_cleanup(
            current,
            &[],
            now(),
            &super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
}

async fn make_source_blob_cleanup_due(
    store: &MetadataStore,
    content_ref: &scope_domain::content_ref::ContentRef,
) {
    entities::source_blob_cleanup_job::Entity::update_many()
        .filter(
            entities::source_blob_cleanup_job::Column::ObjectKey
                .eq(serde_json::to_string(content_ref).unwrap()),
        )
        .col_expr(
            entities::source_blob_cleanup_job::Column::NextRunAtUnix,
            Expr::value(0_i64),
        )
        .exec(store.db.as_ref())
        .await
        .unwrap();
}

fn now() -> u64 {
    1_700_000_000
}

fn blob(sha256: &str) -> SourceBlob {
    SourceBlob {
        content_ref: scope_domain::content_ref::ContentRef::blob_sha256(sha256),
        sha256: sha256.to_string(),
        git_oid: "oid".to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 10,
    }
}
