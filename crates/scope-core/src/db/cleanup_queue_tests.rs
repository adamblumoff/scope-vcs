use crate::db::entities;
use crate::db::{MetadataStore, TestDatabaseTarget};
use crate::domain::store::{DEFAULT_GIT_FILE_MODE, SourceBlob};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};

#[tokio::test]
async fn cleanup_claims_are_bounded_and_failed_work_is_backed_off() {
    let target = TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    let blob = blob("objects/retry-blob");
    store
        .queue_pending_source_blob_deletions(vec![blob.clone()])
        .await
        .unwrap();
    make_source_blob_cleanup_due(&store, &blob.object_key).await;

    let claimed = store.source_blob_cleanup_batch().await.unwrap();
    assert_eq!(claimed.pending, vec![blob.clone()]);
    assert!(
        store
            .source_blob_cleanup_batch()
            .await
            .unwrap()
            .pending
            .is_empty(),
        "an active claim must hide work from concurrent drains"
    );

    store
        .finish_source_blob_cleanup(claimed, std::slice::from_ref(&blob))
        .await
        .unwrap();
    let immediate_retry = store.source_blob_cleanup_batch().await.unwrap();
    assert_eq!(immediate_retry.pending, vec![blob.clone()]);
    store
        .finish_source_blob_cleanup(immediate_retry, std::slice::from_ref(&blob))
        .await
        .unwrap();
    assert!(
        store
            .source_blob_cleanup_batch()
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
    let blob = blob("objects/reclaimed");
    store
        .queue_pending_source_blob_deletions(vec![blob])
        .await
        .unwrap();
    make_source_blob_cleanup_due(&store, "objects/reclaimed").await;
    let stale = store.source_blob_cleanup_batch().await.unwrap();
    make_source_blob_cleanup_due(&store, "objects/reclaimed").await;
    let current = store.source_blob_cleanup_batch().await.unwrap();

    store.finish_source_blob_cleanup(stale, &[]).await.unwrap();
    let row = entities::source_blob_cleanup_job::Entity::find_by_id("objects/reclaimed")
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap();
    assert!(row.completed_at_unix.is_none());
    store
        .finish_source_blob_cleanup(current, &[])
        .await
        .unwrap();
}

async fn make_source_blob_cleanup_due(store: &MetadataStore, object_key: &str) {
    entities::source_blob_cleanup_job::Entity::update_many()
        .filter(entities::source_blob_cleanup_job::Column::ObjectKey.eq(object_key))
        .col_expr(
            entities::source_blob_cleanup_job::Column::NextRunAtUnix,
            Expr::value(0_i64),
        )
        .exec(store.db.as_ref())
        .await
        .unwrap();
}

fn blob(object_key: &str) -> SourceBlob {
    SourceBlob {
        object_key: object_key.to_string(),
        sha256: "sha".to_string(),
        git_oid: "oid".to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 10,
    }
}
