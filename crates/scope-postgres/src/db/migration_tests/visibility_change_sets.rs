use super::*;

#[tokio::test]
async fn visibility_events_are_grouped_and_projection_identity_is_rebuilt() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(24))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('visibility_user', 'visibility', 'visibility@scope.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy
        ) VALUES (
            'visibility/repo', 'visibility', 'repo', 'visibility_user', 'Ready',
            9,
            '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}'::jsonb,
            '{"default_visibility":"Private","rules":[]}'::jsonb
        );
        INSERT INTO scope_visibility_events (
            repo_id, id, ordinal, after_commit_id, source_commit_id,
            author_id, path, old_visibility, new_visibility, current_content
        ) VALUES
            ('visibility/repo', 'vis_1', 0, 'commit-1', NULL, 'visibility_user', '/one', 'Private', 'Public', NULL),
            ('visibility/repo', 'vis_2', 1, 'commit-1', NULL, 'visibility_user', '/two', 'Public', 'Private', NULL),
            ('visibility/repo', 'vis_3', 2, 'commit-1', NULL, 'visibility_user', '/one', 'Public', 'Private', NULL),
            ('visibility/repo', 'vis_4', 3, 'commit-1', 'push-2', 'visibility_user', '/three', 'Private', 'Public', NULL),
            ('visibility/repo', 'vis_5', 4, 'commit-2', 'push-2', 'visibility_user', '/four', 'Public', 'Private', NULL);
        INSERT INTO scope_object_references (object_key, ref_kind, ref_id)
        VALUES ('visibility-object', 'visibility_event', 'visibility/repo:vis_2');
        INSERT INTO scope_projection_read_models (
            repo_id, repo_version, source, audience, rebuilt_at_unix,
            file_count, head_oid, identity_version
        ) VALUES (
            'visibility/repo', 9, 'live', 'public', 1, 0, NULL, 1
        );
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let sets = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, ordinal, anchor_commit_id, source_update_id
             FROM scope_visibility_change_sets
             ORDER BY ordinal"
                .to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(sets.len(), 4);
    assert_eq!(sets[0].try_get::<String>("", "id").unwrap(), "vchg_m0");
    assert_eq!(sets[0].try_get::<i64>("", "ordinal").unwrap(), 0);
    assert_eq!(
        sets[2]
            .try_get::<Option<String>>("", "source_update_id")
            .unwrap()
            .as_deref(),
        Some("push-2")
    );
    assert_eq!(
        sets[3]
            .try_get::<Option<String>>("", "anchor_commit_id")
            .unwrap()
            .as_deref(),
        Some("commit-2")
    );

    let changes = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT change_set_id, ordinal, path, old_visibility, new_visibility
             FROM scope_visibility_changes
             ORDER BY change_set_id, ordinal"
                .to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(changes.len(), 5);
    assert_eq!(changes[0].try_get::<String>("", "path").unwrap(), "/one");
    assert_eq!(changes[1].try_get::<String>("", "path").unwrap(), "/two");
    assert_eq!(
        changes[2].try_get::<String>("", "change_set_id").unwrap(),
        "vchg_m2"
    );
    assert_eq!(changes[2].try_get::<i64>("", "ordinal").unwrap(), 0);
    assert_eq!(changes[2].try_get::<String>("", "path").unwrap(), "/one");
    assert!(!relation_exists(db.as_ref(), "scope_visibility_events").await);

    let reference = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT ref_kind, ref_id FROM scope_object_references
             WHERE object_key = 'visibility-object'"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reference.try_get::<String>("", "ref_kind").unwrap(),
        "visibility_change"
    );
    assert_eq!(
        reference.try_get::<String>("", "ref_id").unwrap(),
        "visibility/repo:vchg_m0:1"
    );

    let projection_count = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM scope_projection_read_models".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(projection_count, 0);
    let rebuild_state = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT state FROM scope_outbox_jobs
             WHERE idempotency_key = 'projection_read_model_rebuild:visibility/repo:9'"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "state")
        .unwrap();
    assert_eq!(rebuild_state, "ready");
}
