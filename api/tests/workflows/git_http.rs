use super::*;

#[tokio::test]
async fn published_receive_pack_accepts_git_push_token() {
    let secret = "scope_git_test";
    let state = test_state_with_git_push_token(secret);
    let mut headers = git_push_token_headers(secret);
    insert_push_intent_header(&state, &mut headers, &test_owner_id(), TEST_PUSH_HEAD_OID).await;

    let access = receive_pack_access(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();

    assert!(matches!(
        access,
        ReceivePackAccess::PublishedMember { author_id, .. } if author_id == test_owner_id()
    ));
}

#[tokio::test]
async fn push_intent_is_signed_instead_of_process_local() {
    let issuer = test_state_with_repo();
    let verifier = test_state_with_repo();
    let token = issuer
        .create_push_intent(
            TEST_REPO_ID,
            &test_owner_id(),
            TEST_PUSH_HEAD_OID,
            repo_config(Visibility::Public),
            repo_config_fingerprint(&repo_config(Visibility::Public)).unwrap(),
            None,
        )
        .unwrap()
        .token;

    let intent = verifier.validate_push_intent_secret(&token).unwrap();
    intent
        .ensure_repo_user(TEST_REPO_ID, &test_owner_id())
        .unwrap();
    let base = intent.base_for_head(TEST_PUSH_HEAD_OID).unwrap();

    assert_eq!(base, None);
}

#[tokio::test]
async fn create_push_intent_hides_repo_before_head_validation_for_non_writer() {
    let state = test_state_with_repo();
    let response = request_push_intent(
        state,
        &bearer_header_for("user_other", "other@example.com"),
        "not-a-git-oid",
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn owner_can_create_first_push_intent_for_unpublished_repo() {
    let state = test_state_with_repo();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .get_mut(TEST_REPO_ID)
            .unwrap()
            .record
            .publication_state = RepoPublicationState::Unpublished;
    }
    let response = request_push_intent(state, &bearer_header(), TEST_PUSH_HEAD_OID).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert!(body["token"].as_str().unwrap().starts_with("scope_pi_"));
    assert!(body["base_head_oid"].is_null());
    assert!(body["expires_at_unix"].as_u64().unwrap() > unix_now());
}

#[tokio::test]
async fn create_push_intent_rejects_sha256_oid_until_git_storage_supports_it() {
    let state = test_state_with_repo();
    let response = request_push_intent(state, &bearer_header(), &"a".repeat(64)).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert_eq!(body["error"], "head_oid must be a full SHA-1 Git object id");
}

fn push_intent_request_json(head_oid: &str) -> String {
    serde_json::json!({
        "head_oid": head_oid,
        "base_config_hash": repo_config_fingerprint(&repo_config(Visibility::Public)).unwrap(),
        "config": repo_config(Visibility::Public),
    })
    .to_string()
}

async fn request_push_intent(state: AppState, authorization: &str, head_oid: &str) -> Response {
    cache_test_jwks(&state);
    router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/push-intents")
                .header(AUTHORIZATION, authorization)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(push_intent_request_json(head_oid)))
                .unwrap(),
        )
        .await
        .unwrap()
}

fn permissioned_git_service(repo: &str, service: &str) -> String {
    format!("/git/permissioned/owner/{repo}/info/refs?service={service}")
}

async fn git_get(app: &axum::Router, uri: String, authorization: Option<&str>) -> Response {
    let mut request = Request::builder().method("GET").uri(uri);
    if let Some(authorization) = authorization {
        request = request.header(AUTHORIZATION, authorization);
    }
    app.clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn receive_pack_rejects_git_push_without_push_intent() {
    let secret = "scope_git_test";
    let state = test_state_with_git_push_token(secret);
    let headers = git_push_token_headers(secret);

    let error = receive_pack_access(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap_err();

    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(error.message(), "valid Scope push intent required");
}

#[tokio::test]
async fn receive_pack_requires_credentials_before_repo_state_is_revealed() {
    let state = test_state_with_repo();
    let app = router(state.clone());

    let existing = git_get(
        &app,
        permissioned_git_service("repo", "git-receive-pack"),
        None,
    )
    .await;
    let missing = git_get(
        &app,
        permissioned_git_service("missing", "git-receive-pack"),
        None,
    )
    .await;

    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
    }
    let pending_publish = git_get(
        &app,
        permissioned_git_service("repo", "git-receive-pack"),
        None,
    )
    .await;

    for response in [existing, missing, pending_publish] {
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().contains_key(WWW_AUTHENTICATE));
    }
}

#[tokio::test]
async fn public_git_remote_cannot_receive_pack() {
    let state = test_state_with_repo();
    let response = git_get(
        &router(state),
        "/git/public/owner/repo/info/refs?service=git-receive-pack".to_string(),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn upload_pack_uses_scope_session_for_owner_projection_after_publish() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
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
            new_content: Some(source_blob("owner only")),
        });

        replace_test_repo(&state, repo);
    }
    let headers = authorization_headers(bearer_header());
    let projection = git_projection_for_request(
        &state,
        &headers,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap();

    assert!(
        projection
            .commits
            .iter()
            .flat_map(|commit| &commit.changes)
            .any(|change| change.path.as_str() == "/secret.txt" && change.new_content.is_some())
    );
}

#[tokio::test]
async fn upload_pack_uses_scope_session_for_member_projection_after_publish() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let member_subject = "user_member";
    let member_id = crate::db::scope_user_id_for_auth_identity("clerk", member_subject);
    {
        let mut repo = repo_with_readme();
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id.clone(),
            RepositoryMemberPermissions::default(),
        ));
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/member-secret.txt").unwrap(),
            ))
            .unwrap();
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/owner-secret.txt").unwrap(),
            ))
            .unwrap();
        repo.graph.commits[0].changes.extend([
            FileChange {
                visibility: Visibility::Private,
                path: ScopePath::parse("/member-secret.txt").unwrap(),
                old_content: None,
                new_content: Some(source_blob("member can read")),
            },
            FileChange {
                visibility: Visibility::Private,
                path: ScopePath::parse("/owner-secret.txt").unwrap(),
                old_content: None,
                new_content: Some(source_blob("owner only")),
            },
        ]);
        replace_test_repo(&state, repo);
    }
    let headers = authorization_headers(bearer_header_for(member_subject, "member@example.com"));
    let projection = git_projection_for_request(
        &state,
        &headers,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap();
    let visible_paths = projection
        .commits
        .iter()
        .flat_map(|commit| &commit.changes)
        .map(|change| change.path.as_str())
        .collect::<Vec<_>>();

    assert!(visible_paths.contains(&"/README.md"));
    assert!(visible_paths.contains(&"/member-secret.txt"));
    assert!(visible_paths.contains(&"/owner-secret.txt"));
}

#[tokio::test]
async fn owner_scope_session_survives_missing_membership_row_for_read() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = repo_with_readme();
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/owner-secret.txt").unwrap(),
            ))
            .unwrap();
        repo.graph.commits[0].changes.push(FileChange {
            visibility: Visibility::Private,
            path: ScopePath::parse("/owner-secret.txt").unwrap(),
            old_content: None,
            new_content: Some(source_blob("owner can read")),
        });
        replace_test_repo(&state, repo);
    }
    let headers = authorization_headers(bearer_header());

    let projection = git_projection_for_request(
        &state,
        &headers,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap();

    assert!(
        projection
            .commits
            .iter()
            .flat_map(|commit| &commit.changes)
            .any(|change| change.path.as_str() == "/owner-secret.txt"
                && change.new_content.is_some())
    );
}

#[tokio::test]
async fn published_receive_pack_accepts_member_scope_session() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let member_subject = "user_member";
    let member_id = crate::db::scope_user_id_for_auth_identity("clerk", member_subject);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.users.insert(
            member_id.clone(),
            test_user(member_id.clone(), "member", "member@example.com"),
        );
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id.clone(),
            member_permissions(true, false, false),
        ));
    }
    let mut headers =
        authorization_headers(bearer_header_for(member_subject, "member@example.com"));
    insert_push_intent_header(&state, &mut headers, &member_id, TEST_PUSH_HEAD_OID).await;

    let access = receive_pack_access(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();

    assert!(matches!(
        access,
        ReceivePackAccess::PublishedMember { author_id, .. } if author_id == member_id
    ));
}

#[tokio::test]
async fn upload_pack_ignores_stale_durable_git_repos() {
    let state = test_state_with_readme();
    cache_test_jwks(&state);
    let raw_repo = owner_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let staged_repo = staged_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    fs::create_dir_all(&raw_repo).unwrap();
    fs::create_dir_all(&staged_repo).unwrap();
    fs::write(raw_repo.join("HEAD"), "not a real source of truth").unwrap();
    fs::write(staged_repo.join("HEAD"), "not a real staged source").unwrap();

    let headers = authorization_headers(bearer_header());

    let repo_path = git_upload_pack_repo_for_request(
        &state,
        &headers,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap();
    let actual = git_stdout_text(
        &repo_path,
        &["show", &format!("{DEFAULT_GIT_BRANCH}:README.md")],
        "read bucket-backed projection",
    )
    .unwrap();

    assert_eq!(actual, "hello");
    let _ = fs::remove_dir_all(raw_repo);
    let _ = fs::remove_dir_all(staged_repo);
}

#[tokio::test]
async fn upload_pack_wrong_basic_credentials_do_not_reveal_repo_existence() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let app = router(state);
    let wrong_basic = format!("Basic {}", BASE64.encode("scope:scope_git_wrong"));

    let existing = git_get(
        &app,
        permissioned_git_service("repo", "git-upload-pack"),
        Some(&wrong_basic),
    )
    .await;
    let missing = git_get(
        &app,
        permissioned_git_service("missing", "git-upload-pack"),
        Some(&wrong_basic),
    )
    .await;
    assert_eq!(existing.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    assert!(existing.headers().contains_key(WWW_AUTHENTICATE));
    assert!(missing.headers().contains_key(WWW_AUTHENTICATE));
    let existing_body = to_bytes(existing.into_body(), 1024 * 1024).await.unwrap();
    let missing_body = to_bytes(missing.into_body(), 1024 * 1024).await.unwrap();

    assert_eq!(existing_body, missing_body);
    assert!(String::from_utf8_lossy(&existing_body).contains("invalid Git credentials"));
    assert!(!String::from_utf8_lossy(&existing_body).contains("owner/repo"));
}

#[tokio::test]
async fn private_upload_pack_without_credentials_challenges_for_auth() {
    let state = test_state_with_repo();
    {
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.graph.commits.push(LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: Vec::new(),
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "initial".to_string(),
            changes: vec![FileChange {
                visibility: Visibility::Private,
                path: ScopePath::parse("/secret.txt").unwrap(),
                old_content: None,
                new_content: Some(source_blob("secret")),
            }],
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let app = router(state);

    let existing = git_get(
        &app,
        permissioned_git_service("repo", "git-upload-pack"),
        None,
    )
    .await;
    let missing = git_get(
        &app,
        permissioned_git_service("missing", "git-upload-pack"),
        None,
    )
    .await;

    for response in [existing, missing] {
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().contains_key(WWW_AUTHENTICATE));
    }
}

#[tokio::test]
async fn unpublished_upload_pack_member_scope_session_stays_hidden() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let member_subject = "user_member";
    let member_id = crate::db::scope_user_id_for_auth_identity("clerk", member_subject);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id,
            RepositoryMemberPermissions::default(),
        ));
    }
    let headers = authorization_headers(bearer_header_for(member_subject, "member@example.com"));

    let error = git_upload_pack_repo_for_request(
        &state,
        &headers,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}
#[tokio::test]
async fn receive_pack_staging_key_does_not_collapse_valid_repo_names() {
    assert_ne!(safe_repo_key("owner", "a-b"), safe_repo_key("owner", "a_b"));
    assert_ne!(safe_repo_key("owner", "a_b"), safe_repo_key("owner", "a.b"));
}

#[tokio::test]
async fn receive_pack_staging_repo_path_is_unique_per_request() {
    let state = test_state_with_repo();
    let first = receive_pack_staging_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let second = receive_pack_staging_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();

    assert_ne!(first, second);
    assert_eq!(first.parent(), second.parent());
    assert_eq!(
        first.parent().and_then(|path| path.file_name()),
        Some(std::ffi::OsStr::new("git-rx"))
    );
    assert!(
        first
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.len() <= 53)
    );
}

#[tokio::test]
async fn first_push_staging_repo_head_points_to_default_branch() {
    let state = test_state_with_repo();
    let staging_repo =
        ensure_first_push_receive_pack_staging_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .unwrap();
    let head = git_stdout_text(
        &staging_repo,
        &["symbolic-ref", "HEAD"],
        "read staging head",
    )
    .unwrap();

    assert_eq!(head.trim(), format!("refs/heads/{DEFAULT_GIT_BRANCH}"));
    let _ = fs::remove_dir_all(staging_repo);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_git_first_push_over_http_applies_immediately() {
    let (state, source, server) = first_push_fixture(
        "real-first-http-push",
        "hello over http\n",
        Some(("script.sh", "#!/bin/sh\necho hi\n")),
    )
    .await;
    run_git(
        Some(&source),
        &["push", "-u", "scope", "HEAD:main"],
        "push first import over http",
    )
    .unwrap();

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::Published
    );
    assert!(repo.first_push_token.is_none());
    let live_tree = repo.live_tree();
    assert!(!live_tree.contains_key(&ScopePath::parse("/.scope/repo.json").unwrap()));
    assert_eq!(repo.repo_config, repo_config(Visibility::Public));
    assert_eq!(
        live_tree
            .get(&ScopePath::parse("/script.sh").unwrap())
            .unwrap()
            .git_file_mode,
        "100755"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chunked_real_git_first_push_over_http_applies_immediately() {
    let (state, source, server) = first_push_fixture(
        "chunked-real-first-http-push",
        "hello over chunked http\n",
        None,
    )
    .await;
    run_git(
        Some(&source),
        &[
            "-c",
            "http.postBuffer=1",
            "push",
            "-u",
            "scope",
            "HEAD:main",
        ],
        "push first import over chunked http",
    )
    .unwrap();

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::Published
    );
    assert!(repo.first_push_token.is_none());
    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("hello over chunked http\n")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chunked_real_git_published_push_over_http_applies_update() {
    let secret = "scope_git_test";
    let state = test_state_with_git_push_token(secret);
    let (origin, server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}").replacen(
        "http://",
        &format!("http://scope:{secret}@"),
        1,
    );
    let public_remote = format!("{origin}/git/public/{TEST_REPO_ID}");
    let source = TempGitRepo(unique_test_path("chunked-real-published-http-push"));
    run_git(
        None,
        &["clone", &public_remote, source.to_str().unwrap()],
        "clone published repo",
    )
    .unwrap();
    run_git(
        Some(&source),
        &["remote", "set-url", "origin", &remote],
        "point origin at permissioned Scope remote",
    )
    .unwrap();

    fs::write(
        source.join("README.md"),
        "hello over published chunked http\n",
    )
    .unwrap();
    run_git(Some(&source), &["add", "-A"], "add readme update").unwrap();
    commit_all(&source, "update readme");
    configure_push_intent_header(&state, &source, &remote, &test_owner_id()).await;
    run_git(
        Some(&source),
        &["-c", "http.postBuffer=1", "push", "origin", "HEAD:main"],
        "push published update over chunked http",
    )
    .unwrap();

    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("hello over published chunked http\n")
    );

    server.abort();
}

async fn first_push_fixture(
    label: &str,
    readme: &str,
    executable: Option<(&str, &str)>,
) -> (AppState, TempGitRepo, tokio::task::JoinHandle<()>) {
    let (state, secret) = test_state_with_first_push_token();
    let (origin, server) = spawn_test_server(&state).await;
    let source = temp_git_repo(label);
    fs::write(source.join("README.md"), readme).unwrap();
    if let Some((path, content)) = executable {
        fs::write(source.join(path), content).unwrap();
    }
    run_git(Some(&source), &["add", "-A"], "add first push files").unwrap();
    if let Some((path, _)) = executable {
        run_git(
            Some(&source),
            &["update-index", "--chmod=+x", path],
            "make first push file executable",
        )
        .unwrap();
    }
    commit_all(&source, "initial");
    let remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}").replacen(
        "http://",
        &format!("http://scope:{secret}@"),
        1,
    );
    run_git(
        Some(&source),
        &["remote", "add", "scope", &remote],
        "add scope remote",
    )
    .unwrap();
    configure_push_intent_header(&state, &source, &remote, &test_owner_id()).await;
    (state, source, server)
}
