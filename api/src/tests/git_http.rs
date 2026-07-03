use super::*;

#[tokio::test]
async fn published_receive_pack_accepts_git_push_token() {
    let state = test_state_with_repo();
    let secret = "scope_git_test";
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.git_push_token = Some(GitPushToken {
            token_hash: git_push_token_hash(secret),
            owner_user_id: repo.record.owner_user_id.clone(),
            created_at_unix: unix_now(),
        });
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
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
        ReceivePackAccess::PublishedMember { author_id, .. } if author_id == test_owner_id()
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
        repo.record.publication_state = RepoPublicationState::Unpublished;
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
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.pending_import = Some(pending_import_fixture(vec![("README.md", "hello")]));
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
async fn receive_pack_hides_pending_import_from_unrelated_scope_user() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.pending_import = Some(pending_import_fixture(vec![("README.md", "hello")]));
    }
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/git/owner/repo/info/refs?service=git-receive-pack")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("user_other", "other@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn receive_pack_reports_pending_import_to_owner_scope_user() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.pending_import = Some(pending_import_fixture(vec![("README.md", "hello")]));
    }
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/git/owner/repo/info/refs?service=git-receive-pack")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn upload_pack_uses_git_push_token_for_owner_projection_after_publish() {
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
            ))
            .unwrap();
        repo.graph.commits[0].changes.push(FileChange {
            visibility: Visibility::Private,
            path: ScopePath::parse("/secret.txt").unwrap(),
            old_content: None,
            new_content: Some(source_blob("owner only")),
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

    assert!(
        projection
            .commits
            .iter()
            .flat_map(|commit| &commit.changes)
            .any(|change| change.path.as_str() == "/secret.txt" && change.new_content.is_some())
    );
}

#[tokio::test]
async fn upload_pack_uses_git_clone_token_for_member_projection_after_publish() {
    let state = test_state_with_repo();
    let member_id = "user_member".to_string();
    let (secret, clone_token) = generate_git_clone_token(&member_id).unwrap();
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
        repo.git_clone_tokens.push(clone_token);

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.users.insert(
            member_id.clone(),
            UserAccount {
                id: member_id.clone(),
                handle: "member".to_string(),
                email: "member@example.com".to_string(),
                email_verified: true,
            },
        );
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
async fn owner_git_credential_survives_missing_membership_row() {
    let state = test_state_with_repo();
    let owner_id = test_owner_id();
    let (secret, owner_token) = generate_git_clone_token(&owner_id).unwrap();
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
        repo.git_clone_tokens.push(owner_token);

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
    let access = receive_pack_access(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
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
    assert!(matches!(
        access,
        ReceivePackAccess::PublishedMember { author_id, .. } if author_id == owner_id
    ));
}

#[tokio::test]
async fn published_receive_pack_accepts_member_git_credential() {
    let state = test_state_with_repo();
    let member_id = "user_member".to_string();
    let (secret, member_token) = generate_git_clone_token(&member_id).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id.clone(),
            member_permissions(true, false, false),
        ));
        repo.git_clone_tokens.push(member_token);
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
        ReceivePackAccess::PublishedMember { author_id, .. } if author_id == member_id
    ));
}

#[tokio::test]
async fn published_receive_pack_rejects_reader_git_credential() {
    let state = test_state_with_repo();
    let member_id = "user_reader".to_string();
    let (secret, member_token) = generate_git_clone_token(&member_id).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id,
            RepositoryMemberPermissions::default(),
        ));
        repo.git_clone_tokens.push(member_token);
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

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn upload_pack_ignores_stale_durable_git_repos() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
    }
    let raw_repo = owner_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let staged_repo = staged_git_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    fs::create_dir_all(&raw_repo).unwrap();
    fs::create_dir_all(&staged_repo).unwrap();
    fs::write(raw_repo.join("HEAD"), "not a real source of truth").unwrap();
    fs::write(staged_repo.join("HEAD"), "not a real staged source").unwrap();

    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, bearer_header().parse().unwrap());

    let repo_path =
        git_upload_pack_repo_for_request(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
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

    let existing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/git/owner/repo/info/refs?service=git-upload-pack")
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
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    for response in [existing, missing] {
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().contains_key(WWW_AUTHENTICATE));
    }
}

#[tokio::test]
async fn unpublished_upload_pack_member_git_credential_stays_hidden() {
    let state = test_state_with_repo();
    let member_id = "user_member".to_string();
    let (secret, member_token) = generate_git_clone_token(&member_id).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.pending_import = Some(pending_import_fixture(vec![("README.md", "hello")]));
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id,
            RepositoryMemberPermissions::default(),
        ));
        repo.git_clone_tokens.push(member_token);
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!("Basic {}", BASE64.encode(format!("scope:{secret}")))
            .parse()
            .unwrap(),
    );

    let error = git_upload_pack_repo_for_request(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
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
        repo.record.publication_state = RepoPublicationState::Unpublished;
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
    fs::write(source.join("script.sh"), "#!/bin/sh\necho hi\n").unwrap();
    run_git(
        Some(&source),
        &["add", "README.md", "script.sh"],
        "add first push files",
    )
    .unwrap();
    run_git(
        Some(&source),
        &["update-index", "--chmod=+x", "script.sh"],
        "make first push script executable",
    )
    .unwrap();
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

    let mut repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::Unpublished
    );
    let pending = repo.pending_import.as_ref().unwrap();
    assert_eq!(pending.default_branch, DEFAULT_GIT_BRANCH);
    assert_eq!(pending.files.len(), 2);
    assert_eq!(pending.files[0].path, "README.md");
    assert_eq!(pending.files[0].mode, "100644");
    assert_eq!(pending.files[1].path, "script.sh");
    assert_eq!(pending.files[1].mode, "100755");
    assert_eq!(pending.files[1].blob.git_file_mode, "100755");
    assert!(
        repo.first_push_token
            .as_ref()
            .unwrap()
            .used_at_unix
            .is_some()
    );

    preview_publish_import(&mut repo).unwrap();
    let live_tree = repo.live_tree();
    assert_eq!(
        live_tree
            .get(&ScopePath::parse("/script.sh").unwrap())
            .unwrap()
            .git_file_mode,
        "100755"
    );

    server.abort();
    let _ = fs::remove_dir_all(source);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chunked_real_git_first_push_over_http_creates_pending_import() {
    let state = test_state_with_repo();
    let (secret, state_for_server) = {
        let (secret, token) = generate_first_push_token(&test_owner_id()).unwrap();
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
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

    let source = temp_git_repo("chunked-real-first-http-push");
    fs::write(source.join("README.md"), "hello over chunked http\n").unwrap();
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

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::Unpublished
    );
    let pending = repo.pending_import.unwrap();
    assert_eq!(pending.default_branch, DEFAULT_GIT_BRANCH);
    assert_eq!(pending.files.len(), 1);
    assert_eq!(pending.files[0].path, "README.md");
    assert!(repo.first_push_token.unwrap().used_at_unix.is_some());

    server.abort();
    let _ = fs::remove_dir_all(source);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chunked_real_git_published_push_over_http_stages_update() {
    let state = test_state_with_repo();
    let secret = "scope_git_test";
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.git_push_token = Some(GitPushToken {
            token_hash: git_push_token_hash(secret),
            owner_user_id: repo.record.owner_user_id.clone(),
            created_at_unix: unix_now(),
        });
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state_for_server = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state_for_server))
            .await
            .unwrap();
    });

    let source = std::env::temp_dir().join(format!(
        "scope-vcs-chunked-real-published-http-push-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&source);
    let remote = format!("http://scope:{secret}@{addr}/git/{TEST_REPO_ID}");
    run_git(
        None,
        &["clone", &remote, source.to_str().unwrap()],
        "clone published repo",
    )
    .unwrap();

    fs::write(
        source.join("README.md"),
        "hello over published chunked http\n",
    )
    .unwrap();
    run_git(Some(&source), &["add", "README.md"], "add readme update").unwrap();
    commit_all(&source, "update readme");
    run_git(
        Some(&source),
        &["-c", "http.postBuffer=1", "push", "origin", "HEAD:main"],
        "push published update over chunked http",
    )
    .unwrap();

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let staged = repo.staged_update.unwrap();
    assert_eq!(staged.branch, format!("refs/heads/{DEFAULT_GIT_BRANCH}"));
    assert_eq!(staged.changes.len(), 1);
    assert_eq!(
        staged.changes[0].path,
        pending_scope_path("/README.md").unwrap()
    );

    server.abort();
    let _ = fs::remove_dir_all(source);
}
