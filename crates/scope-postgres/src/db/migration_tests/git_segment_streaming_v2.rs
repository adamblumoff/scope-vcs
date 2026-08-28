use super::isolated_database;
use crate::migrations;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn v2_cutover_refuses_existing_v1_git_segments() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(32))
        .await
        .unwrap();
    db.execute_unprepared(
        "INSERT INTO scope_users (id, handle, email, email_verified)
         VALUES ('cutover_user', 'cutover-user', 'cutover@scope.test', TRUE);
         INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy
         ) VALUES (
            'cutover-user/repo', 'cutover-user', 'repo', 'cutover_user', 'Ready', 1,
            '{\"kind\":\"scope.repo-config\",\"version\":1,\"visibility\":{\"default\":\"private\",\"rules\":[]}}'::jsonb,
            '{\"default_visibility\":\"Private\",\"rules\":[]}'::jsonb
         );
         INSERT INTO scope_git_segments (
            repo_id, first_sequence, last_sequence, geometric_tier,
            base_oid, head_oid, object_key, sha256, size_bytes
         ) VALUES (
            'cutover-user/repo', 1, 1, 0, NULL, repeat('a', 40),
            '{\"GitSegmentSha256\":\"old\"}', 'old', 3
         );",
    )
    .await
    .unwrap();

    let error = migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("requires scope_git_segments to be empty")
    );
    assert!(!table_exists(db.as_ref(), "scope_git_segment_uploads").await);

    db.execute_unprepared("DELETE FROM scope_git_segments")
        .await
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();
    assert!(table_exists(db.as_ref(), "scope_git_segment_uploads").await);
}

#[tokio::test]
async fn v2_cutover_replaces_generic_object_columns_with_segment_identity() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();

    let columns = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT column_name
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND table_name = 'scope_git_segments'
             ORDER BY column_name"
                .to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "column_name").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        [
            "base_oid",
            "first_sequence",
            "geometric_tier",
            "head_oid",
            "last_sequence",
            "repo_id",
            "segment_id",
        ]
    );
}

async fn table_exists<C>(db: &C, table: &str) -> bool
where
    C: ConnectionTrait,
{
    db.query_one(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT to_regclass(format('%I.%I', current_schema(), $1)) IS NOT NULL AS present",
        [table.into()],
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get::<bool>("", "present")
    .unwrap()
}
