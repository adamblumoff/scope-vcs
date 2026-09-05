use super::*;

#[tokio::test]
async fn history_entry_positions_migration_preserves_rows_and_allows_repeated_sources() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(37))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('history-owner', 'history-owner', 'history@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy, incarnation_id
        ) VALUES (
            'history-owner/repo', 'history-owner', 'repo', 'history-owner', 'Ready',
            1, '{}'::jsonb, '{}'::jsonb, 'repoi_history'
        );
        INSERT INTO scope_repository_history_views (
            repo_id, audience, repo_version, generation, identity_version,
            available, visible_files, head_oid
        ) VALUES ('history-owner/repo', 'public', 1, 'generation', 1, TRUE, TRUE, NULL);
        INSERT INTO scope_repository_history_entries (repo_id, audience, position, source_id, payload)
        VALUES ('history-owner/repo', 'public', 0, 'same-push', '{"fragment":"first"}');
        "#,
    ).await.unwrap();

    let repeated_fragment =
        "INSERT INTO scope_repository_history_entries (repo_id, audience, position, source_id, payload)
         VALUES ('history-owner/repo', 'public', 1, 'same-push', '{\"fragment\":\"second\"}')";
    let previous_error = db.execute_unprepared(repeated_fragment).await.unwrap_err();
    assert!(
        previous_error
            .to_string()
            .contains("scope_repository_history_entries_pkey")
    );

    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(plan.pending.len(), 1);
    assert_eq!(plan.pending[0].name, "m0038_history_entry_positions");
    assert_eq!(plan.pending[0].impact, MigrationImpact::MaintenanceRequired);
    assert!(migrations::apply_online(db.as_ref()).await.is_err());
    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();
    migrations::assert_exact_state(db.as_ref()).await.unwrap();

    db.execute_unprepared(repeated_fragment).await.unwrap();
    let rows = db.query_all(Statement::from_string(
        DatabaseBackend::Postgres,
        "SELECT payload->>'fragment' AS fragment FROM scope_repository_history_entries ORDER BY position".to_string(),
    )).await.unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row.try_get::<String>("", "fragment").unwrap())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert!(db.execute_unprepared(
        "INSERT INTO scope_repository_history_entries (repo_id, audience, position, source_id, payload)
         VALUES ('history-owner/repo', 'public', 1, 'different-push', '{}')",
    ).await.is_err());
    assert!(relation_exists(db.as_ref(), "idx_scope_repository_history_entries_source").await);

    db.execute_unprepared("DELETE FROM scope_repository_history_views")
        .await
        .unwrap();
    let count = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM scope_repository_history_entries".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(count, 0);
}
