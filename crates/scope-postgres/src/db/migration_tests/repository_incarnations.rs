use super::isolated_database;
use crate::migrations;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn repository_incarnation_cutover_backfills_live_and_cleanup_rows() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(33))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('user_incarnation', 'incarnation-owner', 'owner@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy
        ) VALUES (
            'incarnation-owner/repo', 'incarnation-owner', 'repo',
            'user_incarnation', 'Ready', 1,
            '{"visibility":{"default":"Private","rules":[]},"history":{"rewrites":[]}}'::jsonb,
            '{"default_visibility":"Private","rules":[]}'::jsonb
        );
        INSERT INTO scope_repo_storage_cleanup_jobs (
            repo_id, generation, owner_handle, repo_name, attempts,
            next_run_at_unix, last_error, completed_at_unix,
            created_at_unix, updated_at_unix
        ) VALUES (
            'deleted-owner/repo', 'generation', 'deleted-owner', 'repo',
            0, 1, NULL, NULL, 1, 1
        );
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    for table in ["scope_repositories", "scope_repo_storage_cleanup_jobs"] {
        let row = db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("SELECT incarnation_id FROM {table} LIMIT 1"),
            ))
            .await
            .unwrap()
            .unwrap();
        let incarnation = row.try_get::<String>("", "incarnation_id").unwrap();
        assert!(incarnation.starts_with("repoi_m0034_"));
    }
}
