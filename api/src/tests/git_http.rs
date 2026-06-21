use super::*;

#[tokio::test]
async fn published_receive_pack_accepts_basic_git_push_token() {
    let state = test_state_with_repo();
    let secret = "scope_git_test";
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.git_push_token = Some(GitPushToken {
            token_hash: git_push_token_hash(secret),
            owner_user_id: repo.record.owner_user_id.clone(),
            created_at_unix: unix_now(),
        });
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!("Basic {}", BASE64.encode(format!("scope:{secret}")))
            .parse()
            .unwrap(),
    );

    let access = receive_pack_access(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();

    assert!(matches!(
        access,
        ReceivePackAccess::PublishedOwner { author_id } if author_id == test_owner_id()
    ));
}

#[tokio::test]
async fn receive_pack_requires_credentials_before_repo_state_is_revealed() {
    let state = test_state_with_repo();
    let app = router(state.clone());

    let existing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/git/owner/repo/info/refs?service=git-receive-pack")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/git/owner/missing/info/refs?service=git-receive-pack")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingPublish;
    }
    let pending_publish = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/git/owner/repo/info/refs?service=git-receive-pack")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    for response in [existing, missing, pending_publish] {
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().contains_key(WWW_AUTHENTICATE));
    }
}

#[tokio::test]
async fn receive_pack_reports_pending_publish_only_after_owner_token_auth() {
    let state = test_state_with_repo();
    let secret = "scope_git_test";
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        repo.git_push_token = Some(GitPushToken {
            token_hash: git_push_token_hash(secret),
            owner_user_id: repo.record.owner_user_id.clone(),
            created_at_unix: unix_now(),
        });
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!("Basic {}", BASE64.encode(format!("scope:{secret}")))
            .parse()
            .unwrap(),
    );

    let error = receive_pack_access(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap_err();

    assert_eq!(error.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn upload_pack_accepts_basic_git_push_token_for_owner_projection() {
    let state = test_state_with_repo();
    let secret = "scope_git_test";
    {
        let mut repo = repo_with_readme();
        repo.git_push_token = Some(GitPushToken {
            token_hash: git_push_token_hash(secret),
            owner_user_id: repo.record.owner_user_id.clone(),
            created_at_unix: unix_now(),
        });
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/secret.txt").unwrap(),
                [repo.record.owner_user_id.clone()],
            ))
            .unwrap();
        repo.graph.commits[0].changes.push(FileChange {
            path: ScopePath::parse("/secret.txt").unwrap(),
            old_content: None,
            new_content: Some("owner only".to_string()),
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!("Basic {}", BASE64.encode(format!("scope:{secret}")))
            .parse()
            .unwrap(),
    );

    let projection = git_projection_for_request(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let git = build_virtual_git_projection(&projection);

    assert!(git.blobs.iter().any(|blob| blob.path == "/secret.txt"));
}

#[tokio::test]
async fn upload_pack_uses_owner_raw_git_repo_when_present() {
    let state = test_state_with_repo();
    let secret = "scope_git_test";
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.git_push_token = Some(GitPushToken {
            token_hash: git_push_token_hash(secret),
            owner_user_id: repo.record.owner_user_id.clone(),
            created_at_unix: unix_now(),
        });
    }
    let source = temp_git_repo("owner-raw-upload-pack");
    fs::write(source.join("README.md"), "raw owner").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add raw readme").unwrap();
    commit_all(&source, "raw owner commit");
    let bare = std::env::temp_dir().join(format!(
        "scope-vcs-owner-raw-upload-pack-bare-{}-{}",
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
        "clone owner raw repo",
    )
    .unwrap();
    let raw_repo = owner_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    replace_git_repo(&bare, &raw_repo).unwrap();
    let expected =
        git_stdout_text(&raw_repo, &["rev-parse", DEFAULT_GIT_BRANCH], "raw head").unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!("Basic {}", BASE64.encode(format!("scope:{secret}")))
            .parse()
            .unwrap(),
    );

    let repo_path =
        git_upload_pack_repo_for_request(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap();
    let actual = git_stdout_text(
        &repo_path,
        &["rev-parse", DEFAULT_GIT_BRANCH],
        "advertised owner raw head",
    )
    .unwrap();

    assert_eq!(actual, expected);
    let staged_source = temp_git_repo("owner-staged-upload-pack");
    fs::write(staged_source.join("README.md"), "staged owner").unwrap();
    run_git(
        Some(&staged_source),
        &["add", "README.md"],
        "add staged readme",
    )
    .unwrap();
    commit_all(&staged_source, "staged owner commit");
    let staged_bare = std::env::temp_dir().join(format!(
        "scope-vcs-owner-staged-upload-pack-bare-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&staged_bare);
    run_git(
        None,
        &[
            "clone",
            "--bare",
            staged_source.to_str().unwrap(),
            staged_bare.to_str().unwrap(),
        ],
        "clone owner staged repo",
    )
    .unwrap();
    let staged_repo = staged_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    replace_git_repo(&staged_bare, &staged_repo).unwrap();
    let expected_staged = git_stdout_text(
        &staged_repo,
        &["rev-parse", DEFAULT_GIT_BRANCH],
        "staged raw head",
    )
    .unwrap();

    let repo_path =
        git_upload_pack_repo_for_request(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap();
    let actual_staged = git_stdout_text(
        &repo_path,
        &["rev-parse", DEFAULT_GIT_BRANCH],
        "advertised owner staged head",
    )
    .unwrap();

    assert_eq!(actual_staged, expected_staged);
    let _ = fs::remove_dir_all(&source);
    let _ = fs::remove_dir_all(&staged_source);
    let _ = fs::remove_dir_all(raw_repo);
    let _ = fs::remove_dir_all(staged_repo);
}

#[tokio::test]
async fn upload_pack_wrong_basic_credentials_do_not_reveal_repo_existence() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let app = router(state);
    let wrong_basic = format!("Basic {}", BASE64.encode("scope:scope_git_wrong"));

    let existing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/git/owner/repo/info/refs?service=git-upload-pack")
                .header(AUTHORIZATION, wrong_basic.as_str())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let missing = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/git/owner/missing/info/refs?service=git-upload-pack")
                .header(AUTHORIZATION, wrong_basic)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
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
        repo.policy = Policy::new(Visibility::Private, &repo.record.owner_user_id);
        repo.graph.commits.push(LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: Vec::new(),
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "initial".to_string(),
            mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
            changes: vec![FileChange {
                path: ScopePath::parse("/secret.txt").unwrap(),
                old_content: None,
                new_content: Some("secret".to_string()),
            }],
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/git/owner/repo/info/refs?service=git-upload-pack")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().contains_key(WWW_AUTHENTICATE));
}
#[test]
fn receive_pack_staging_key_does_not_collapse_valid_repo_names() {
    assert_ne!(safe_repo_key("owner", "a-b"), safe_repo_key("owner", "a_b"));
    assert_ne!(safe_repo_key("owner", "a_b"), safe_repo_key("owner", "a.b"));
}

#[test]
fn receive_pack_staging_repo_path_is_unique_per_request() {
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

#[test]
fn first_push_staging_repo_head_points_to_default_branch() {
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
async fn real_git_first_push_over_http_creates_pending_import() {
    let state = test_state_with_repo();
    let (secret, state_for_server) = {
        let (secret, token) = generate_first_push_token(&test_owner_id()).unwrap();
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingFirstPush;
        repo.first_push_token = Some(token);
        (secret, state.clone())
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state_for_server))
            .await
            .unwrap();
    });

    let source = temp_git_repo("real-first-http-push");
    fs::write(source.join("README.md"), "hello over http\n").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add readme").unwrap();
    commit_all(&source, "initial");

    let remote = format!("http://scope:{secret}@{addr}/git/{TEST_REPO_ID}");
    run_git(
        Some(&source),
        &["remote", "add", "scope", &remote],
        "add scope remote",
    )
    .unwrap();
    run_git(
        Some(&source),
        &["push", "-u", "scope", "HEAD:main"],
        "push first import over http",
    )
    .unwrap();

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::PendingPublish
    );
    let pending = repo.pending_import.unwrap();
    assert_eq!(pending.default_branch, DEFAULT_GIT_BRANCH);
    assert_eq!(pending.files.len(), 1);
    assert_eq!(pending.files[0].path, "README.md");
    assert!(repo.first_push_token.unwrap().used_at_unix.is_some());

    server.abort();
    let _ = fs::remove_dir_all(source);
}

#[test]
fn pushed_tree_rejects_gitlinks_instead_of_dropping_them() {
    let repo = temp_git_repo("gitlink-test");
    fs::write(repo.join("README.md"), "hello").unwrap();
    run_git(Some(&repo), &["add", "README.md"], "add readme").unwrap();
    commit_all(&repo, "initial");
    let commit = git_stdout_text(&repo, &["rev-parse", "HEAD"], "read head")
        .unwrap()
        .trim()
        .to_string();
    run_git(
        Some(&repo),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{commit},vendor/submodule"),
        ],
        "add gitlink",
    )
    .unwrap();
    commit_all(&repo, "add gitlink");

    let error = git_tree_files(&repo, "HEAD").unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert!(error.message.contains("unsupported Git tree entry"));
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn pushed_tree_rejects_non_utf8_blobs_before_pending_import() {
    let repo = temp_git_repo("binary-test");
    fs::write(repo.join("image.bin"), [0xff, 0x00, 0x61]).unwrap();
    run_git(Some(&repo), &["add", "image.bin"], "add binary").unwrap();
    commit_all(&repo, "binary");

    let error = git_tree_files(&repo, "HEAD").unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert!(error.message.contains("valid UTF-8 text"));
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn pushed_tree_rejects_modes_that_projection_cannot_preserve() {
    let repo = temp_git_repo("mode-test");
    fs::write(repo.join("script.sh"), "#!/bin/sh\necho hi\n").unwrap();
    run_git(Some(&repo), &["add", "script.sh"], "add script").unwrap();
    run_git(
        Some(&repo),
        &["update-index", "--chmod=+x", "script.sh"],
        "make script executable",
    )
    .unwrap();
    commit_all(&repo, "executable");

    let error = git_tree_files(&repo, "HEAD").unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert!(error.message.contains("unsupported Git file mode"));
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn pushed_tree_rejects_paths_scope_would_normalize_or_git_cannot_serve() {
    validate_pushed_file_path("docs/read me.md").unwrap();
    for path in [
        "README.md ",
        "dir\\file.txt",
        "line\nbreak.txt",
        "./README.md",
        "docs/../README.md",
    ] {
        let error = validate_pushed_file_path(path).unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }
}
