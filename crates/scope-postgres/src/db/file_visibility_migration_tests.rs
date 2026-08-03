use super::migration_tests::{initialize_ready_v6, isolated_database};
use crate::migrations;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn repository_visibility_becomes_file_policy_and_readiness() {
    let (_target, db, _lease) = isolated_database().await;
    initialize_ready_v6(db.as_ref()).await;
    // The adopted v6 schema has no SeaQL ledger yet, so these nine steps apply
    // m0001 through m0009 and leave m0010 pending for the assertion below.
    migrations::Migrator::up(db.as_ref(), Some(9))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
            INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('user_visibility_migration', 'visibility-migration', 'visibility@scope.test', TRUE);
            INSERT INTO scope_repositories (
                id, owner_handle, name, owner_user_id, publication_state,
                default_visibility, change_version, repo_config, policy
            ) VALUES
                (
                    'repo_ready', 'visibility-migration', 'ready',
                    'user_visibility_migration', 'Published', 'Private', 1,
                    '{"kind":"scope.repo-config","version":1,"visibility":{"default":"public","rules":[]}}'::jsonb,
                    '{"default_visibility":"Public","rules":[]}'::jsonb
                ),
                (
                    'repo_awaiting', 'visibility-migration', 'awaiting',
                    'user_visibility_migration', 'Unpublished', 'Public', 1,
                    '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}'::jsonb,
                    '{"default_visibility":"Private","rules":[]}'::jsonb
                );
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let states = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "
                SELECT id, publication_state,
                       repo_config #>> '{visibility,default}' AS config_default,
                       policy ->> 'default_visibility' AS policy_default
                FROM scope_repositories
                ORDER BY id
            "
            .to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "id").unwrap(),
                row.try_get::<String>("", "publication_state").unwrap(),
                row.try_get::<String>("", "config_default").unwrap(),
                row.try_get::<String>("", "policy_default").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        [
            (
                "repo_awaiting".to_string(),
                "AwaitingFirstPush".to_string(),
                "private".to_string(),
                "Private".to_string(),
            ),
            (
                "repo_ready".to_string(),
                "Ready".to_string(),
                "public".to_string(),
                "Public".to_string(),
            ),
        ]
    );
    let visibility_column_count = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "
                SELECT count(*) AS count
                FROM information_schema.columns
                WHERE table_schema = current_schema()
                  AND table_name = 'scope_repositories'
                  AND column_name = 'default_visibility'
            "
            .to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(visibility_column_count, 0);
}
