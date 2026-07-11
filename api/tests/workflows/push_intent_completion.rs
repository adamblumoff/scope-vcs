use super::*;
use crate::domain::repo_config::RepoConfigVisibilityRule;

async fn post(state: AppState, uri: &str, authorization: String, body: String) -> Response {
    router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(AUTHORIZATION, authorization)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn owner_post(state: AppState, uri: &str, body: String) -> Response {
    post(
        state,
        uri,
        bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
        body,
    )
    .await
}

async fn mint_intent(state: AppState, head_oid: &str, config: RepoConfig) -> String {
    let response = owner_post(
        state,
        "/v1/repos/owner/repo/push-intents",
        push_intent_request_json(head_oid, config),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await["token"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn stored_config(state: &AppState) -> RepoConfig {
    find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .repo_config
}

fn bare_clone(source: &FsPath, label: &str) -> TempGitRepo {
    let bare = TempGitRepo(std::env::temp_dir().join(format!(
        "scope-vcs-{label}-{}-{}",
        std::process::id(),
        unix_now()
    )));
    let _ = fs::remove_dir_all(bare.as_ref());
    run_git(
        None,
        &[
            "clone",
            "--bare",
            source.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
        "clone test bare repository",
    )
    .unwrap();
    bare
}

fn push_intent_request_json(head_oid: &str, config: RepoConfig) -> String {
    push_intent_request_json_with_base(
        head_oid,
        repo_config_fingerprint(&repo_config(Visibility::Public)).unwrap(),
        config,
    )
}

fn push_intent_request_json_with_base(
    head_oid: &str,
    base_config_hash: String,
    config: RepoConfig,
) -> String {
    serde_json::json!({
        "head_oid": head_oid,
        "base_config_hash": base_config_hash,
        "config": config,
    })
    .to_string()
}

fn readme_private_config() -> RepoConfig {
    let mut config = repo_config(Visibility::Public);
    config.visibility.rules.push(RepoConfigVisibilityRule {
        path: "/README.md".into(),
        visibility: ConfigVisibility::Private,
    });
    config
}

async fn published_git_fixture(label: &str) -> (AppState, TempGitRepo, String) {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo(label);
    fs::write(source.join("README.md"), "hello\n").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add readme").unwrap();
    commit_all(&source, "initial");
    let bare = bare_clone(&source, &format!("{label}-bare"));
    let head = git_head_oid(&bare);
    apply_first_push_from_staging_repo(&state, &bare, repo_config(Visibility::Public)).await;
    (state, source, head)
}

#[tokio::test]
async fn create_push_intent_rejects_stale_local_config_base_hash() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    state
        .metadata
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.record.default_visibility = Visibility::Private;
            repo.policy = Policy::new(Visibility::Private);
            repo.repo_config = repo_config(Visibility::Private);
        })
        .await
        .unwrap();

    let response = owner_post(
        state.clone(),
        "/v1/repos/owner/repo/push-intents",
        push_intent_request_json_with_base(
            TEST_PUSH_HEAD_OID,
            repo_config_fingerprint(&repo_config(Visibility::Public)).unwrap(),
            readme_private_config(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await["error"],
        "repo config changed since review; rerun scope review"
    );

    assert_eq!(
        stored_config(&state).await,
        repo_config(Visibility::Private)
    );
}

#[tokio::test]
async fn create_push_intent_rejects_oversized_config_for_git_header_transport() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let mut oversized_config = repo_config(Visibility::Public);
    oversized_config.visibility.rules = (0..300)
        .map(|index| RepoConfigVisibilityRule {
            path: format!("/private/path-{index}.txt"),
            visibility: ConfigVisibility::Private,
        })
        .collect::<Vec<_>>();

    let response = owner_post(
        state,
        "/v1/repos/owner/repo/push-intents",
        push_intent_request_json(TEST_PUSH_HEAD_OID, oversized_config),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response_json(response).await["error"]
            .as_str()
            .unwrap()
            .contains("repo config exceeds")
    );
}

#[tokio::test]
async fn complete_push_intent_rejects_stale_config_only_review() {
    let (state, _source, head_oid) = published_git_fixture("stale-config-complete").await;

    let old_token = mint_intent(state.clone(), &head_oid, repo_config(Visibility::Private)).await;

    let readme_private_config = readme_private_config();
    let new_token = mint_intent(state.clone(), &head_oid, readme_private_config.clone()).await;

    let applied = owner_post(
        state.clone(),
        "/v1/repos/owner/repo/push-intents/complete",
        serde_json::json!({ "token": new_token }).to_string(),
    )
    .await;
    assert_eq!(applied.status(), StatusCode::OK);
    assert_eq!(response_json(applied).await["config_applied"], true);

    let stale = owner_post(
        state.clone(),
        "/v1/repos/owner/repo/push-intents/complete",
        serde_json::json!({ "token": old_token }).to_string(),
    )
    .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(stored_config(&state).await, readme_private_config);
}

#[tokio::test]
async fn complete_push_intent_rejects_content_changed_since_review() {
    let (state, _source, head_oid) = published_git_fixture("stale-content-complete").await;
    let base_config = stored_config(&state).await;
    let token = state
        .create_push_intent(
            TEST_REPO_ID,
            &test_owner_id(),
            &head_oid,
            readme_private_config(),
            repo_config_fingerprint(&base_config).unwrap(),
            Some("stale-git-snapshot-key".to_string()),
        )
        .unwrap()
        .token;

    let response = owner_post(
        state.clone(),
        "/v1/repos/owner/repo/push-intents/complete",
        serde_json::json!({ "token": token }).to_string(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await["error"],
        "repo content changed since review; rerun scope push"
    );
    assert_eq!(stored_config(&state).await, repo_config(Visibility::Public));
}

#[tokio::test]
async fn complete_push_intent_hides_repo_before_token_claim_mismatch() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let other_subject = "user_other";
    let other_id = crate::db::scope_user_id_for_auth_identity("clerk", other_subject);
    let base_config = repo_config(Visibility::Public);
    let token = state
        .create_push_intent(
            "other/repo",
            &other_id,
            TEST_PUSH_HEAD_OID,
            base_config.clone(),
            repo_config_fingerprint(&base_config).unwrap(),
            None,
        )
        .unwrap()
        .token;

    let response = post(
        state,
        "/v1/repos/owner/repo/push-intents/complete",
        bearer_header_for(other_subject, "other@example.com"),
        serde_json::json!({ "token": token }).to_string(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn content_push_rejects_stale_reviewed_config() {
    let (state, source, _head_oid) = published_git_fixture("stale-config-content-push").await;

    fs::write(source.join("README.md"), "content from old review\n").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add stale update").unwrap();
    commit_all(&source, "stale content update");
    let stale_bare = bare_clone(&source, "stale-config-content-update-bare");
    let update = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &stale_bare,
        &test_owner_id(),
        repo_config(Visibility::Public),
    )
    .await
    .unwrap();

    let newer_config = readme_private_config();
    state
        .metadata
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            crate::domain::reviewed_updates::apply_reviewed_config_to_repo(
                repo,
                crate::domain::reviewed_updates::ReviewedConfigUpdateInput {
                    author_id: test_owner_id(),
                    config: newer_config.clone(),
                },
            )
            .unwrap();
        })
        .await
        .unwrap();

    let error = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        &test_owner_id(),
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(
        error.message(),
        "repo config changed since review; rerun scope push"
    );
    assert_eq!(stored_config(&state).await, newer_config);
}
