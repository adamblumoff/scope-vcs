use super::*;

const PUSH_ONLY_MEMBER_ID: &str = "user_push_only";
const DEFAULT_PUBLIC_CONFIG_JSON: &str = r#"{
    "kind": "scope.repo-config",
    "version": 1,
    "visibility": { "default": "public", "rules": [] },
    "history": { "rewrites": [] }
}"#;
const PRIVATE_SECRET_CONFIG_JSON: &[u8] = br#"{
    "kind": "scope.repo-config",
    "version": 1,
    "visibility": {
        "default": "public",
        "rules": [{ "path": "/secret.txt", "visibility": "private" }]
    },
    "history": { "rewrites": [] }
}"#;

fn config_with_rules(default: Visibility, rules: &[(&str, ConfigVisibility)]) -> RepoConfig {
    let mut config = repo_config(default);
    config.visibility.rules = rules
        .iter()
        .map(
            |(path, visibility)| crate::domain::repo_config::RepoConfigVisibilityRule {
                path: (*path).to_string(),
                visibility: *visibility,
            },
        )
        .collect();
    config
}

fn install_push_only_repo(state: &AppState, mut repo: StoredRepository) {
    repo.members.push(test_repository_member(
        TEST_REPO_ID,
        PUSH_ONLY_MEMBER_ID,
        member_permissions(true, false, true),
    ));
    lock_catalog(state)
        .unwrap()
        .repositories
        .insert(TEST_REPO_ID.to_string(), repo);
}

async fn push_as_push_only_member(
    state: &AppState,
    changes: Vec<(&str, Option<&str>)>,
    config: RepoConfig,
) -> Result<PersistedReceivePackUpdate, crate::error::ApiError> {
    let mut update = receive_pack_update(changes);
    update.base_config_hash = repo_config_fingerprint(
        &find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await?
            .repo_config,
    )?;
    update.config = config;
    persist_receive_pack_update_and_promote(
        state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        PUSH_ONLY_MEMBER_ID,
    )
    .await
}

fn repo_with_docs_config(
    config: &RepoConfig,
    config_json: &str,
    message: &str,
) -> StoredRepository {
    let mut repo = test_repo(&test_owner_id());
    repo.record.default_visibility = Visibility::Private;
    repo.repo_config = config.clone();
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
        message: message.to_string(),
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
    repo
}

fn repo_with_private_secret() -> StoredRepository {
    let mut repo = repo_with_readme();
    repo.repo_config = RepoConfig::parse_json(PRIVATE_SECRET_CONFIG_JSON).unwrap();
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
    repo
}

#[tokio::test]
async fn push_only_member_cannot_publish_private_path_via_config() {
    let state = test_state_with_repo();
    let existing_config = config_with_rules(
        Visibility::Private,
        &[("/README.md", ConfigVisibility::Public)],
    );
    let mut repo = repo_with_readme();
    repo.record.default_visibility = Visibility::Private;
    repo.repo_config = existing_config;
    repo.policy = Policy::new(Visibility::Private);
    repo.policy
        .add_rule(VisibilityRule::public(
            ScopePath::parse("/README.md").unwrap(),
        ))
        .unwrap();
    install_push_only_repo(&state, repo);

    let config = config_with_rules(
        Visibility::Private,
        &[
            ("/README.md", ConfigVisibility::Public),
            ("/secret.txt", ConfigVisibility::Public),
        ],
    );

    let error = push_as_push_only_member(&state, vec![("/secret.txt", Some("leak"))], config)
        .await
        .unwrap_err();

    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(error.message(), "file visibility permission required");
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert!(
        !repo
            .live_tree()
            .contains_key(&ScopePath::parse("/secret.txt").unwrap())
    );
}

#[tokio::test]
async fn push_only_member_cannot_restore_stale_public_config_after_visibility_change() {
    let state = test_state_with_repo();
    let readme_path = ScopePath::parse("/README.md").unwrap();
    let mut repo = repo_with_readme();
    crate::domain::repo_actions::set_visibility(
        &mut repo,
        &test_owner_id(),
        std::slice::from_ref(&readme_path),
        Visibility::Private,
    )
    .unwrap();
    assert_eq!(
        repo.repo_config.visibility_for_path(&readme_path),
        Visibility::Private
    );
    install_push_only_repo(&state, repo);

    let error = push_as_push_only_member(
        &state,
        vec![("/README.md", Some("member update"))],
        repo_config(Visibility::Public),
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(error.message(), "file visibility permission required");
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(
        repo.policy.effective_visibility(&readme_path),
        Visibility::Private
    );
    assert_eq!(
        repo.repo_config.visibility_for_path(&readme_path),
        Visibility::Private
    );
}

#[tokio::test]
async fn push_only_member_cannot_persist_non_visibility_config_metadata() {
    let state = test_state_with_repo();
    install_push_only_repo(&state, repo_with_readme());

    let mut config = repo_config(Visibility::Public);
    config.schema = Some("https://scope.example/schema.json".to_string());
    let error =
        push_as_push_only_member(&state, vec![("/README.md", Some("member update"))], config)
            .await
            .unwrap_err();

    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(error.message(), "file visibility permission required");
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(repo.repo_config.schema, None);
}

#[tokio::test]
async fn push_only_member_cannot_bootstrap_future_public_config_rule() {
    let state = test_state_with_repo();
    let config = config_with_rules(
        Visibility::Private,
        &[("/future-public/**", ConfigVisibility::Public)],
    );
    let config_json = serde_json::to_string(&config).unwrap();
    let mut repo = test_repo(&test_owner_id());
    repo.record.default_visibility = Visibility::Private;
    repo.repo_config = repo_config(Visibility::Private);
    repo.policy = Policy::new(Visibility::Private);
    install_push_only_repo(&state, repo);

    let error = push_as_push_only_member(
        &state,
        vec![("/.scope/repo.json", Some(&config_json))],
        config,
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(error.message(), "file visibility permission required");
}

#[tokio::test]
async fn push_only_member_cannot_erase_existing_private_policy_with_default_config() {
    let state = test_state_with_repo();
    install_push_only_repo(&state, repo_with_private_secret());

    let error = push_as_push_only_member(
        &state,
        vec![("/.scope/repo.json", Some(DEFAULT_PUBLIC_CONFIG_JSON))],
        RepoConfig::parse_json(DEFAULT_PUBLIC_CONFIG_JSON.as_bytes()).unwrap(),
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(error.message(), "file visibility permission required");
}

#[tokio::test]
async fn push_only_member_cannot_weaken_deleted_private_path_during_config_bootstrap() {
    let state = test_state_with_repo();
    install_push_only_repo(&state, repo_with_private_secret());

    let error = push_as_push_only_member(
        &state,
        vec![
            ("/secret.txt", None),
            ("/.scope/repo.json", Some(DEFAULT_PUBLIC_CONFIG_JSON)),
        ],
        RepoConfig::parse_json(DEFAULT_PUBLIC_CONFIG_JSON.as_bytes()).unwrap(),
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(error.message(), "file visibility permission required");
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert!(
        repo.live_tree()
            .contains_key(&ScopePath::parse("/secret.txt").unwrap())
    );
}

#[tokio::test]
async fn push_only_member_can_add_file_under_existing_public_config_glob() {
    let state = test_state_with_repo();
    let config = config_with_rules(
        Visibility::Private,
        &[("/docs/**", ConfigVisibility::Public)],
    );
    let config_json = serde_json::to_string(&config).unwrap();
    install_push_only_repo(
        &state,
        repo_with_docs_config(&config, &config_json, "initial docs config"),
    );

    let persisted =
        push_as_push_only_member(&state, vec![("/docs/new.md", Some("new docs"))], config)
            .await
            .unwrap();

    assert_eq!(persisted, PersistedReceivePackUpdate::Applied);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(
        repo.live_tree()
            .get(&ScopePath::parse("/docs/new.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("new docs")
    );
}

#[tokio::test]
async fn unchanged_rewrite_entry_does_not_block_push_only_member_content_push() {
    let state = test_state_with_repo();
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
    install_push_only_repo(
        &state,
        repo_with_docs_config(&config, config_json, "initial docs config with rewrite"),
    );

    let persisted =
        push_as_push_only_member(&state, vec![("/docs/new.md", Some("new docs"))], config)
            .await
            .unwrap();

    assert_eq!(persisted, PersistedReceivePackUpdate::Applied);
}
