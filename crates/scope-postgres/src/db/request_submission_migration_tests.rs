use super::migration_tests::{initialize_ready_v6, isolated_database};
use crate::migrations;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn review_cycles_migrate_to_one_submission_and_terminal_facts() {
    let (_target, db, _lease) = isolated_database().await;
    initialize_ready_v6(db.as_ref()).await;
    migrations::Migrator::up(db.as_ref(), Some(7))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
            INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('user_request_migration', 'request-migration', 'request-migration@scope.test', TRUE);
            INSERT INTO scope_repositories (
                id, owner_handle, name, owner_user_id, publication_state,
                default_visibility, change_version, repo_config, policy
            ) VALUES (
                'repo_request_migration', 'request-migration', 'repo',
                'user_request_migration', 'Published', 'Public', 1, '{}'::jsonb, '{}'::jsonb
            );
            INSERT INTO scope_requests (
                id, repo_id, name, author_user_id, author_role, audience,
                base_main_oid, head_oid, git_snapshot, title, description_markdown,
                state, activity_version, first_ready_at_unix, ready_at_unix,
                completed_at_unix, completed_by_user_id,
                merged_at_unix, merged_by_user_id, merged_head_oid, merged_main_oid,
                created_at_unix, updated_at_unix
            ) VALUES
                (
                    'request_draft', 'repo_request_migration', 'draft', 'user_request_migration',
                    'Owner', 'Public', repeat('a', 40), repeat('b', 40), NULL, 'Draft', '',
                    'Working', 1, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 1, 1
                ),
                (
                    'request_reopened', 'repo_request_migration', 'reopened', 'user_request_migration',
                    'Owner', 'Public', repeat('a', 40), repeat('b', 40), NULL, 'Reopened', '',
                    'Working', 5, 2, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 1, 5
                ),
                (
                    'request_open', 'repo_request_migration', 'open', 'user_request_migration',
                    'Owner', 'Public', repeat('a', 40), repeat('b', 40), NULL, 'Open', '',
                    'ReadyForReview', 4, 2, 4, NULL, NULL, NULL, NULL, NULL, NULL, 1, 4
                ),
                (
                    'request_closed', 'repo_request_migration', 'closed', 'user_request_migration',
                    'Owner', 'Public', repeat('a', 40), repeat('b', 40), NULL, 'Closed', '',
                    'Completed', 3, 2, NULL, 3, 'user_request_migration',
                    NULL, NULL, NULL, NULL, 1, 3
                ),
                (
                    'request_merged', 'repo_request_migration', 'merged', 'user_request_migration',
                    'Owner', 'Public', repeat('a', 40), repeat('b', 40), NULL, 'Merged', '',
                    'Completed', 3, 2, NULL, 3, 'user_request_migration',
                    3, 'user_request_migration', repeat('b', 40), repeat('c', 40), 1, 3
                );
            INSERT INTO scope_request_events (
                id, request_id, actor_user_id, kind, position, payload, created_at_unix
            ) VALUES
                ('event_reopened_submit_1', 'request_reopened', 'user_request_migration',
                 'ReadyForReview', 2, '{"ReadyForReview":{"head_oid":"first"}}', 2),
                ('event_reopened_return_1', 'request_reopened', 'user_request_migration',
                 'ReturnedToWorking', 3, '{"ReturnedToWorking":{"head_oid":"first","reason":"RevisionPushed"}}', 3),
                ('event_reopened_submit_2', 'request_reopened', 'user_request_migration',
                 'ReadyForReview', 4, '{"ReadyForReview":{"head_oid":"second"}}', 4),
                ('event_reopened_return_2', 'request_reopened', 'user_request_migration',
                 'ReturnedToWorking', 5, '{"ReturnedToWorking":{"head_oid":"second","reason":"AuthorReturned"}}', 5);
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let facts = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, submitted_at_unix, closed_at_unix, closed_by_user_id, merged_at_unix
             FROM scope_requests ORDER BY id"
                .to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "id").unwrap(),
                row.try_get::<Option<i64>>("", "submitted_at_unix").unwrap(),
                row.try_get::<Option<i64>>("", "closed_at_unix").unwrap(),
                row.try_get::<Option<String>>("", "closed_by_user_id")
                    .unwrap(),
                row.try_get::<Option<i64>>("", "merged_at_unix").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        facts,
        vec![
            (
                "request_closed".to_string(),
                Some(2),
                Some(3),
                Some("user_request_migration".to_string()),
                None,
            ),
            ("request_draft".to_string(), None, None, None, None),
            ("request_merged".to_string(), Some(2), None, None, Some(3)),
            ("request_open".to_string(), Some(2), None, None, None),
            ("request_reopened".to_string(), Some(2), None, None, None),
        ]
    );
    let events = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT kind, position, payload FROM scope_request_events
             WHERE request_id = 'request_reopened' ORDER BY position"
                .to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].try_get::<String>("", "kind").unwrap(),
        "Submitted"
    );
    assert_eq!(events[0].try_get::<i64>("", "position").unwrap(), 2);
    let payload = events[0]
        .try_get::<serde_json::Value>("", "payload")
        .unwrap();
    assert_eq!(payload["Submitted"]["head_oid"], "first");
}
