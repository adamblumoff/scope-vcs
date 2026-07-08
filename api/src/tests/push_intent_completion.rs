use super::*;

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
    RepoConfig::parse_json(
        br#"{
            "kind": "scope.repo-config",
            "version": 1,
            "visibility": {
                "default": "public",
                "rules": [
                    { "path": "/README.md", "visibility": "private" }
                ]
            }
        }"#,
    )
    .unwrap()
}

#[tokio::test]
async fn create_push_intent_uses_server_config_as_saved_local_config_base() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let desired_config = repo_config(Visibility::Private);

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/push-intents")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(push_intent_request_json(
                    TEST_PUSH_HEAD_OID,
                    desired_config,
                )))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response_json(response).await["token"].is_string());
}

#[tokio::test]
async fn create_push_intent_rejects_stale_local_config_base_hash() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.repo_config = repo_config(Visibility::Private);
    }

    let app = router(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/push-intents")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(push_intent_request_json_with_base(
                    TEST_PUSH_HEAD_OID,
                    repo_config_fingerprint(&repo_config(Visibility::Public)).unwrap(),
                    readme_private_config(),
                )))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await["error"],
        "repo config changed since review; rerun scope review"
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/config")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(
        body["config"],
        serde_json::json!(repo_config(Visibility::Private))
    );
    assert_eq!(
        body["config_hash"],
        repo_config_fingerprint(&repo_config(Visibility::Private)).unwrap()
    );
}

#[tokio::test]
async fn create_push_intent_rejects_oversized_config_for_git_header_transport() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let rules = (0..300)
        .map(|index| {
            serde_json::json!({
                "path": format!("/private/path-{index}.txt"),
                "visibility": "private",
            })
        })
        .collect::<Vec<_>>();
    let oversized_config = RepoConfig::parse_json(
        serde_json::json!({
            "kind": "scope.repo-config",
            "version": 1,
            "visibility": {
                "default": "public",
                "rules": rules,
            },
            "history": {
                "rewrites": [],
            },
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/push-intents")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(push_intent_request_json(
                    TEST_PUSH_HEAD_OID,
                    oversized_config,
                )))
                .unwrap(),
        )
        .await
        .unwrap();

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
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo("stale-config-complete");
    fs::write(source.join("README.md"), "hello\n").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add readme").unwrap();
    commit_all(&source, "initial");
    let bare = std::env::temp_dir().join(format!(
        "scope-vcs-stale-config-complete-bare-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&bare);
    run_git(
        None,
        &[
            "clone",
            "--bare",
            source.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
        "clone stale config complete bare repo",
    )
    .unwrap();
    let head_oid = git_stdout_text(&bare, &["rev-parse", DEFAULT_GIT_BRANCH], "read head")
        .unwrap()
        .trim()
        .to_string();
    let pending =
        pending_import_from_staging_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME, &bare).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.pending_import = Some(pending);
        preview_publish_import(repo).unwrap();
    }

    let app = router(state.clone());
    let old_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/push-intents")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(push_intent_request_json(
                    &head_oid,
                    repo_config(Visibility::Private),
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old_response.status(), StatusCode::OK);
    let old_token = response_json(old_response).await["token"]
        .as_str()
        .unwrap()
        .to_string();

    let readme_private_config = readme_private_config();
    let new_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/push-intents")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(push_intent_request_json(
                    &head_oid,
                    readme_private_config.clone(),
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(new_response.status(), StatusCode::OK);
    let new_token = response_json(new_response).await["token"]
        .as_str()
        .unwrap()
        .to_string();

    let applied = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/push-intents/complete")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "token": new_token }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(applied.status(), StatusCode::OK);
    assert_eq!(response_json(applied).await["config_applied"], true);

    let stale = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/push-intents/complete")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "token": old_token }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert_eq!(repo.repo_config, readme_private_config);
    let _ = fs::remove_dir_all(source);
    let _ = fs::remove_dir_all(bare);
}

#[tokio::test]
async fn complete_push_intent_rejects_content_changed_since_review() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo("stale-content-complete");
    fs::write(source.join("README.md"), "hello\n").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add readme").unwrap();
    commit_all(&source, "initial");
    let bare = std::env::temp_dir().join(format!(
        "scope-vcs-stale-content-complete-bare-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&bare);
    run_git(
        None,
        &[
            "clone",
            "--bare",
            source.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
        "clone stale content complete bare repo",
    )
    .unwrap();
    let head_oid = git_stdout_text(&bare, &["rev-parse", DEFAULT_GIT_BRANCH], "read head")
        .unwrap()
        .trim()
        .to_string();
    let pending =
        pending_import_from_staging_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME, &bare).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.pending_import = Some(pending);
        preview_publish_import(repo).unwrap();
    }
    let base_config = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .unwrap()
        .repo_config;
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

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/push-intents/complete")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "token": token }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await["error"],
        "repo content changed since review; rerun scope push"
    );
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert_eq!(repo.repo_config, repo_config(Visibility::Public));
    let _ = fs::remove_dir_all(source);
    let _ = fs::remove_dir_all(bare);
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

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/push-intents/complete")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(other_subject, "other@example.com"),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "token": token }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn content_push_rejects_stale_reviewed_config() {
    let state = test_state_with_repo();
    let source = temp_git_repo("stale-config-content-push");
    fs::write(source.join("README.md"), "hello\n").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add readme").unwrap();
    commit_all(&source, "initial");
    let bare = std::env::temp_dir().join(format!(
        "scope-vcs-stale-config-content-base-bare-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&bare);
    run_git(
        None,
        &[
            "clone",
            "--bare",
            source.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
        "clone stale config content base repo",
    )
    .unwrap();
    let pending =
        pending_import_from_staging_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME, &bare).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.pending_import = Some(pending);
        preview_publish_import(repo).unwrap();
    }

    fs::write(source.join("README.md"), "content from old review\n").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add stale update").unwrap();
    commit_all(&source, "stale content update");
    let stale_bare = std::env::temp_dir().join(format!(
        "scope-vcs-stale-config-content-update-bare-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&stale_bare);
    run_git(
        None,
        &[
            "clone",
            "--bare",
            source.to_str().unwrap(),
            stale_bare.to_str().unwrap(),
        ],
        "clone stale config content update repo",
    )
    .unwrap();
    let update = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &stale_bare,
        &test_owner_id(),
        repo_config(Visibility::Public),
    )
    .unwrap();

    let newer_config = readme_private_config();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        crate::domain::reviewed_updates::apply_reviewed_config_to_repo(
            repo,
            crate::domain::reviewed_updates::ReviewedConfigUpdateInput {
                author_id: test_owner_id(),
                config: newer_config.clone(),
            },
        )
        .unwrap();
    }

    let error = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        &test_owner_id(),
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::CONFLICT);
    assert_eq!(
        error.message,
        "repo config changed since review; rerun scope push"
    );
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert_eq!(repo.repo_config, newer_config);
    let _ = fs::remove_dir_all(source);
    let _ = fs::remove_dir_all(bare);
    let _ = fs::remove_dir_all(stale_bare);
}
