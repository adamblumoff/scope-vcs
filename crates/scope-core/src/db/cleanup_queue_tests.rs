use super::cleanup_queue::{
    complete_pending_repo_storage_cleanup_at, complete_pending_source_blob_cleanup_at,
    queue_pending_repo_storage_cleanup_row, queue_pending_source_blob_deletion_rows,
};
use crate::domain::store::{DEFAULT_GIT_FILE_MODE, RepoStorageCleanup, SourceBlob};
use sea_orm::{DbBackend, MockDatabase, MockExecResult};

#[tokio::test]
async fn source_blob_cleanup_queue_writes_typed_rows_not_metadata_lock_json() {
    let db = MockDatabase::new(DbBackend::Postgres)
        .append_exec_results(vec![inserted()])
        .into_connection();
    queue_pending_source_blob_deletion_rows(
        &db,
        [SourceBlob {
            object_key: "objects/blob-1".to_string(),
            sha256: "sha".to_string(),
            git_oid: "oid".to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 10,
        }],
    )
    .await
    .unwrap();

    let sql = transaction_sql(db);
    assert!(
        sql.iter()
            .any(|statement| statement.contains("scope_source_blob_cleanup_jobs"))
    );
    assert!(
        sql.iter().any(|statement| {
            statement.contains("ON CONFLICT") && statement.contains("DO UPDATE")
        }),
        "cleanup queue writes must use atomic upsert: {sql:?}"
    );
    assert!(
        !sql.iter()
            .any(|statement| statement.contains("scope_metadata_locks"))
    );
}

#[tokio::test]
async fn repo_storage_cleanup_queue_writes_typed_rows_not_metadata_lock_json() {
    let db = MockDatabase::new(DbBackend::Postgres)
        .append_exec_results(vec![inserted()])
        .into_connection();
    queue_pending_repo_storage_cleanup_row(
        &db,
        RepoStorageCleanup {
            owner_handle: "owner".to_string(),
            repo_name: "repo".to_string(),
        },
    )
    .await
    .unwrap();

    let sql = transaction_sql(db);
    assert!(
        sql.iter()
            .any(|statement| statement.contains("scope_repo_storage_cleanup_jobs"))
    );
    assert!(
        sql.iter().any(|statement| {
            statement.contains("ON CONFLICT") && statement.contains("DO UPDATE")
        }),
        "cleanup queue writes must use atomic upsert: {sql:?}"
    );
    assert!(
        !sql.iter()
            .any(|statement| statement.contains("scope_metadata_locks"))
    );
}

#[tokio::test]
async fn repo_storage_cleanup_completion_is_generation_fenced() {
    let db = MockDatabase::new(DbBackend::Postgres)
        .append_exec_results(vec![MockExecResult::default()])
        .into_connection();
    complete_pending_repo_storage_cleanup_at(&db, "owner/repo", "cleanup-generation", 10)
        .await
        .unwrap();

    let sql = transaction_sql(db);
    assert!(
        sql.iter()
            .any(|statement| statement.contains("\"generation\"")),
        "completion must filter by cleanup row generation: {sql:?}"
    );
}

#[tokio::test]
async fn source_blob_cleanup_completion_is_generation_fenced() {
    let db = MockDatabase::new(DbBackend::Postgres)
        .append_exec_results(vec![MockExecResult::default()])
        .into_connection();
    complete_pending_source_blob_cleanup_at(&db, "objects/blob-1", "cleanup-generation", 10)
        .await
        .unwrap();

    let sql = transaction_sql(db);
    assert!(
        sql.iter()
            .any(|statement| statement.contains("\"generation\"")),
        "completion must filter by cleanup row generation: {sql:?}"
    );
}

fn transaction_sql(db: sea_orm::DatabaseConnection) -> Vec<String> {
    db.into_transaction_log()
        .into_iter()
        .flat_map(|transaction| {
            transaction
                .statements()
                .iter()
                .map(|statement| statement.sql.clone())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn inserted() -> MockExecResult {
    MockExecResult {
        last_insert_id: 1,
        rows_affected: 1,
    }
}
