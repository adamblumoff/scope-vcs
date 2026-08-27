use super::{isolated_database, relation_exists};
use crate::migrations;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn promotes_replied_threads_and_deletes_empty_synthetics() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(11))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
            INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('user', 'user', 'user@example.com', TRUE);
            INSERT INTO scope_repositories (
                id, owner_handle, name, owner_user_id, publication_state,
                change_version, repo_config, policy
            ) VALUES ('repo', 'user', 'repo', 'user', 'Ready', 1, '{}', '{}');
            INSERT INTO scope_requests (
                id, repo_id, name, author_user_id, author_role, audience,
                base_main_oid, head_oid, git_snapshot, title, description_markdown,
                activity_version, submitted_at_unix, closed_at_unix, closed_by_user_id,
                merged_at_unix, merged_by_user_id, merged_head_oid, merged_main_oid,
                created_at_unix, updated_at_unix
            ) VALUES (
                'request', 'repo', 'request', 'user', 'Owner', 'Private',
                'base', 'head', NULL, 'Request', '', 5,
                NULL, NULL, NULL, NULL, NULL, NULL, NULL, 1, 5
            );
            INSERT INTO scope_request_change_blocks (
                id, request_id, position, actor_user_id, old_head_oid,
                new_head_oid, git_snapshot, created_at_unix
            ) VALUES
                ('revision_empty', 'request', 2, 'user', 'base', 'one', '{}', 2),
                ('revision_replied', 'request', 3, 'user', 'one', 'head', '{}', 3);
            INSERT INTO scope_request_discussions (
                id, request_id, opened_position, last_activity_position,
                author_user_id, subject, body_markdown, status,
                client_discussion_id, created_at_unix, resolved_at_unix,
                resolved_by_user_id
            ) VALUES
                ('empty', 'request', 2, 2, 'user',
                 '{"ChangeBlock":{"change_block_id":"revision_empty"}}',
                 NULL, 'Dormant', 'change-block:empty', 2, NULL, NULL),
                ('replied', 'request', 3, 5, 'user',
                 '{"ChangeBlock":{"change_block_id":"revision_replied"}}',
                 NULL, 'Open', 'change-block:replied', 3, NULL, NULL);
            INSERT INTO scope_request_discussion_replies (
                id, discussion_id, position, depth, author_user_id,
                body_markdown, reply_to_reply_id, client_reply_id, created_at_unix
            ) VALUES
                ('root', 'replied', 4, 0, 'user', 'Root comment', NULL, 'root-client', 4),
                ('child', 'replied', 5, 1, 'user', 'Child comment', 'root', 'child-client', 5);
            INSERT INTO scope_request_discussion_read_states (
                discussion_id, user_id, read_through_position, updated_at_unix
            ) VALUES ('empty', 'user', 2, 2), ('replied', 'user', 5, 5);
            INSERT INTO scope_object_references (object_key, ref_kind, ref_id)
            VALUES ('object', 'request_change_block_snapshot', 'revision_replied');
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    assert!(!relation_exists(db.as_ref(), "scope_request_change_blocks").await);
    assert!(relation_exists(db.as_ref(), "scope_request_revisions").await);
    let discussions = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, opened_position, body_markdown, revision_id FROM scope_request_discussions ORDER BY id".to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(discussions.len(), 1);
    assert_eq!(
        discussions[0].try_get::<String>("", "id").unwrap(),
        "replied"
    );
    assert_eq!(
        discussions[0]
            .try_get::<i64>("", "opened_position")
            .unwrap(),
        4
    );
    assert_eq!(
        discussions[0]
            .try_get::<String>("", "body_markdown")
            .unwrap(),
        "Root comment"
    );
    assert_eq!(
        discussions[0].try_get::<String>("", "revision_id").unwrap(),
        "revision_replied"
    );
    let reply = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, reply_to_reply_id FROM scope_request_discussion_replies".to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply.try_get::<String>("", "id").unwrap(), "child");
    assert_eq!(
        reply
            .try_get::<Option<String>>("", "reply_to_reply_id")
            .unwrap(),
        None
    );
    let schema = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT
                NOT EXISTS (
                    SELECT 1 FROM information_schema.columns
                    WHERE table_schema = current_schema()
                      AND table_name = 'scope_request_discussion_replies'
                      AND column_name = 'depth'
                ) AS depth_removed,
                to_regclass('idx_scope_request_discussion_replies_chronological') IS NOT NULL
                    AS chronological_index_exists"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert!(schema.try_get::<bool>("", "depth_removed").unwrap());
    assert!(
        schema
            .try_get::<bool>("", "chronological_index_exists")
            .unwrap()
    );
    let reference = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT ref_kind FROM scope_object_references WHERE ref_id = 'revision_replied'"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reference.try_get::<String>("", "ref_kind").unwrap(),
        "request_revision_snapshot"
    );
}
