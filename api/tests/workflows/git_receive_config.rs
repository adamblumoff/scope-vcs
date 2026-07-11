use super::*;

const PUSH_ONLY_MEMBER_ID: &str = "user_push_only";
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

async fn install_push_only_repo(state: &AppState, mut repo: StoredRepository) {
    repo.members.push(test_repository_member(
        TEST_REPO_ID,
        PUSH_ONLY_MEMBER_ID,
        member_permissions(true, false, true),
    ));
    replace_test_repo(state, repo).await;
}

async fn push_as_push_only_member(
    state: &AppState,
    changes: Vec<(&str, Option<&str>)>,
    config: RepoConfig,
) -> Result<PersistedReceivePackUpdate, crate::error::ApiError> {
    let mut update = receive_pack_update(state, changes);
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

async fn rejected_config_push(
    state: &AppState,
    changes: Vec<(&str, Option<&str>)>,
    config: RepoConfig,
) -> StoredRepository {
    let error = push_as_push_only_member(state, changes, config)
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(error.message(), "file visibility permission required");
    find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
}

#[tokio::test]
async fn push_only_member_cannot_publish_private_path_via_config() {
    let state = test_state_with_repo();
    let existing_config = config_with_rules(
        Visibility::Private,
        &[("/README.md", ConfigVisibility::Public)],
    );
    let mut repo = repo_with_readme(&state);
    repo.record.default_visibility = Visibility::Private;
    repo.repo_config = existing_config;
    repo.policy = Policy::new(Visibility::Private);
    repo.policy
        .add_rule(VisibilityRule::public(
            ScopePath::parse("/README.md").unwrap(),
        ))
        .unwrap();
    install_push_only_repo(&state, repo).await;

    let config = config_with_rules(
        Visibility::Private,
        &[
            ("/README.md", ConfigVisibility::Public),
            ("/secret.txt", ConfigVisibility::Public),
        ],
    );

    let repo = rejected_config_push(&state, vec![("/secret.txt", Some("leak"))], config).await;
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
    let mut repo = repo_with_readme(&state);
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
    install_push_only_repo(&state, repo).await;

    let repo = rejected_config_push(
        &state,
        vec![("/README.md", Some("member update"))],
        repo_config(Visibility::Public),
    )
    .await;
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
    install_push_only_repo(&state, repo_with_readme(&state)).await;

    let mut config = repo_config(Visibility::Public);
    config.schema = Some("https://scope.example/schema.json".to_string());
    let repo =
        rejected_config_push(&state, vec![("/README.md", Some("member update"))], config).await;
    assert_eq!(repo.repo_config.schema, None);
}
