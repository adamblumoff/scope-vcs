use super::*;

#[tokio::test]
async fn existing_git_heads_get_one_due_compaction_job() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(23))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('scheduler_user', 'scheduler', 'scheduler@scope.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy
        ) VALUES (
            'scheduler/repo', 'scheduler', 'repo', 'scheduler_user', 'Ready',
            7,
            '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}'::jsonb,
            '{"default_visibility":"Private","rules":[]}'::jsonb
        );
        INSERT INTO scope_git_heads (
            repo_id, head_oid, push_sequence, change_version,
            manifest_object_key, manifest_sha256, manifest_size_bytes
        ) VALUES (
            'scheduler/repo', repeat('a', 40), 7, 7,
            '{"GitManifestSha256":"manifest"}', repeat('b', 64), 10
        );
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let job = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT repo_id, target_sequence, attempts, lease_owner
             FROM scope_git_compaction_jobs"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        job.try_get::<String>("", "repo_id").unwrap(),
        "scheduler/repo"
    );
    assert_eq!(job.try_get::<i64>("", "target_sequence").unwrap(), 7);
    assert_eq!(job.try_get::<i32>("", "attempts").unwrap(), 0);
    assert_eq!(
        job.try_get::<Option<String>>("", "lease_owner").unwrap(),
        None
    );
}
