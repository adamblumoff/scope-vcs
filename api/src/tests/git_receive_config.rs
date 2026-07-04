use super::*;

#[test]
fn push_only_member_cannot_publish_private_path_via_config() {
    let state = test_state_with_repo();
    let member_id = "user_push_only";
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.policy
            .add_rule(VisibilityRule::public(
                ScopePath::parse("/README.md").unwrap(),
            ))
            .unwrap();
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id,
            member_permissions(true, false, true),
        ));
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let mut update = receive_pack_update(vec![("/secret.txt", Some("leak"))]);
    update.config = RepoConfig::parse_json(
        br#"{
            "kind": "scope.repo-config",
            "version": 1,
            "visibility": {
                "default": "private",
                "rules": [
                    { "path": "/README.md", "visibility": "public" },
                    { "path": "/secret.txt", "visibility": "public" }
                ]
            },
            "history": {
                "rewrites": []
            }
        }"#,
    )
    .unwrap();

    let error = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        member_id,
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert_eq!(error.message, "file visibility permission required");
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert!(
        !repo
            .live_tree()
            .contains_key(&ScopePath::parse("/secret.txt").unwrap())
    );
}

#[test]
fn push_only_member_cannot_bootstrap_future_public_config_rule() {
    let state = test_state_with_repo();
    let member_id = "user_push_only";
    let config_json = r#"{
        "kind": "scope.repo-config",
        "version": 1,
        "visibility": {
            "default": "private",
            "rules": [
                { "path": "/future-public/**", "visibility": "public" }
            ]
        },
        "history": {
            "rewrites": []
        }
    }"#;
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id,
            member_permissions(true, false, true),
        ));
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let mut update = receive_pack_update(vec![("/.scope/repo.json", Some(config_json))]);
    update.config = RepoConfig::parse_json(config_json.as_bytes()).unwrap();

    let error = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        member_id,
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert_eq!(error.message, "file visibility permission required");
}

#[test]
fn push_only_member_cannot_erase_existing_private_policy_with_default_config() {
    let state = test_state_with_repo();
    let member_id = "user_push_only";
    let config_json = r#"{
        "kind": "scope.repo-config",
        "version": 1,
        "visibility": {
            "default": "public",
            "rules": []
        },
        "history": {
            "rewrites": []
        }
    }"#;
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/secret.txt").unwrap(),
            ))
            .unwrap();
        repo.graph.commits[0].changes.push(FileChange {
            visibility: Visibility::Private,
            path: ScopePath::parse("/secret.txt").unwrap(),
            old_content: None,
            new_content: Some(source_blob("secret")),
        });
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id,
            member_permissions(true, false, true),
        ));
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let mut update = receive_pack_update(vec![("/.scope/repo.json", Some(config_json))]);
    update.config = RepoConfig::parse_json(config_json.as_bytes()).unwrap();

    let error = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        member_id,
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert_eq!(error.message, "file visibility permission required");
}

#[test]
fn push_only_member_cannot_weaken_deleted_private_path_during_config_bootstrap() {
    let state = test_state_with_repo();
    let member_id = "user_push_only";
    let config_json = r#"{
        "kind": "scope.repo-config",
        "version": 1,
        "visibility": {
            "default": "public",
            "rules": []
        },
        "history": {
            "rewrites": []
        }
    }"#;
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/secret.txt").unwrap(),
            ))
            .unwrap();
        repo.graph.commits[0].changes.push(FileChange {
            visibility: Visibility::Private,
            path: ScopePath::parse("/secret.txt").unwrap(),
            old_content: None,
            new_content: Some(source_blob("secret")),
        });
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id,
            member_permissions(true, false, true),
        ));
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let mut update = receive_pack_update(vec![
        ("/secret.txt", None),
        ("/.scope/repo.json", Some(config_json)),
    ]);
    update.config = RepoConfig::parse_json(config_json.as_bytes()).unwrap();

    let error = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        member_id,
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert_eq!(error.message, "file visibility permission required");
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert!(
        repo.live_tree()
            .contains_key(&ScopePath::parse("/secret.txt").unwrap())
    );
}

#[test]
fn push_only_member_can_add_file_under_existing_public_config_glob() {
    let state = test_state_with_repo();
    let member_id = "user_push_only";
    let config_json = r#"{
        "kind": "scope.repo-config",
        "version": 1,
        "visibility": {
            "default": "private",
            "rules": [
                { "path": "/docs/**", "visibility": "public" }
            ]
        },
        "history": {
            "rewrites": []
        }
    }"#;
    let config = RepoConfig::parse_json(config_json.as_bytes()).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.policy
            .add_rule(VisibilityRule::public(
                ScopePath::parse("/docs/existing.md").unwrap(),
            ))
            .unwrap();
        repo.graph.commits.push(LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: Vec::new(),
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "initial docs config".to_string(),
            changes: vec![
                FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/docs/existing.md").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("existing docs")),
                },
                FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/.scope/repo.json").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob(config_json)),
                },
            ],
        });
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id,
            member_permissions(true, false, true),
        ));
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let mut update = receive_pack_update(vec![("/docs/new.md", Some("new docs"))]);
    update.config = config;

    let persisted = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        member_id,
    )
    .unwrap();

    assert_eq!(persisted, PersistedReceivePackUpdate::Applied);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert_eq!(
        repo.live_tree()
            .get(&ScopePath::parse("/docs/new.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("new docs")
    );
}

#[test]
fn unchanged_rewrite_entry_does_not_block_push_only_member_content_push() {
    let state = test_state_with_repo();
    let member_id = "user_push_only";
    let config_json = r#"{
        "kind": "scope.repo-config",
        "version": 1,
        "visibility": {
            "default": "private",
            "rules": [
                { "path": "/docs/**", "visibility": "public" }
            ]
        },
        "history": {
            "rewrites": [
                {
                    "path": "/leaked.txt",
                    "action": "redact-public-history"
                }
            ]
        }
    }"#;
    let config = RepoConfig::parse_json(config_json.as_bytes()).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.policy
            .add_rule(VisibilityRule::public(
                ScopePath::parse("/docs/existing.md").unwrap(),
            ))
            .unwrap();
        repo.graph.commits.push(LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: Vec::new(),
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "initial docs config with rewrite".to_string(),
            changes: vec![
                FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/docs/existing.md").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("existing docs")),
                },
                FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/.scope/repo.json").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob(config_json)),
                },
            ],
        });
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id,
            member_permissions(true, false, true),
        ));
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let mut update = receive_pack_update(vec![("/docs/new.md", Some("new docs"))]);
    update.config = config;

    let persisted = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        member_id,
    )
    .unwrap();

    assert_eq!(persisted, PersistedReceivePackUpdate::Applied);
}
