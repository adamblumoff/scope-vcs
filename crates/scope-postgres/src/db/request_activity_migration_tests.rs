use super::migration_tests::{initialize_ready_v6, isolated_database};
use crate::migrations;
use scope_domain::requests::{RequestEventPayload, request_identity_audit_fact};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn started_event_payloads_migrate_to_bounded_identity_facts() {
    let (_target, db, _lease) = isolated_database().await;
    initialize_ready_v6(db.as_ref()).await;
    migrations::Migrator::up(db.as_ref(), Some(10))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
            INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('user_started_migration', 'started-migration', 'started@scope.test', TRUE);
            INSERT INTO scope_repositories (
                id, owner_handle, name, owner_user_id, publication_state,
                change_version, repo_config, policy
            ) VALUES (
                'repo_started_migration', 'started-migration', 'repo',
                'user_started_migration', 'Ready', 1, '{}'::jsonb, '{}'::jsonb
            );
            INSERT INTO scope_requests (
                id, repo_id, name, author_user_id, author_role, audience,
                base_main_oid, head_oid, git_snapshot, title, description_markdown,
                activity_version, submitted_at_unix, closed_at_unix, closed_by_user_id,
                merged_at_unix, merged_by_user_id, merged_head_oid, merged_main_oid,
                created_at_unix, updated_at_unix
            ) VALUES (
                'request_started_migration', 'repo_started_migration', 'bounded',
                'user_started_migration', 'Owner', 'Public', repeat('a', 40), repeat('b', 40),
                NULL, 'Initial title', 'hello 🌍', 2, 2, NULL, NULL, NULL, NULL, NULL, NULL, 1, 2
            );
            INSERT INTO scope_request_events (
                id, request_id, actor_user_id, kind, position, payload, created_at_unix
            ) VALUES
                (
                    'event_started_migration', 'request_started_migration',
                    'user_started_migration', 'Started', 1,
                    '{"Started":{"title":"Initial title","description_markdown":"hello 🌍"}}', 1
                ),
                (
                    'event_submitted_migration', 'request_started_migration',
                    'user_started_migration', 'Submitted', 2,
                    '{"Submitted":{"head_oid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}', 2
                );
            INSERT INTO scope_request_events (
                id, request_id, actor_user_id, kind, position, payload, created_at_unix
            )
            SELECT
                'event_started_batch_' || lpad(sequence::text, 3, '0'),
                'request_started_migration',
                'user_started_migration',
                'Started',
                sequence + 2,
                jsonb_build_object(
                    'Started',
                    jsonb_build_object(
                        'title', 'Batch title ' || sequence,
                        'description_markdown', 'Batch description ' || sequence
                    )
                ),
                sequence + 2
            FROM generate_series(1, 101) AS sequence;
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let rows = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT kind, payload FROM scope_request_events ORDER BY position".to_string(),
        ))
        .await
        .unwrap();
    let started = rows[0].try_get::<serde_json::Value>("", "payload").unwrap();
    let started = serde_json::from_value::<RequestEventPayload>(started).unwrap();
    assert_eq!(
        started,
        RequestEventPayload::Started {
            identity: request_identity_audit_fact("Initial title", "hello 🌍").unwrap(),
        }
    );
    assert_eq!(rows[1].try_get::<String>("", "kind").unwrap(), "Submitted");
    assert_eq!(
        rows[1].try_get::<serde_json::Value>("", "payload").unwrap(),
        serde_json::json!({
            "Submitted": {
                "head_oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            }
        })
    );
    let compact_started_count = rows
        .iter()
        .filter(|row| row.try_get::<String>("", "kind").unwrap() == "Started")
        .filter(|row| {
            row.try_get::<serde_json::Value>("", "payload")
                .unwrap()
                .pointer("/Started/identity")
                .is_some()
        })
        .count();
    assert_eq!(compact_started_count, 102);
}
