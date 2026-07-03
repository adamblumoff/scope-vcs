use super::*;

#[tokio::test]
async fn private_projection_cache_key_is_shared_by_owner_and_member() {
    let state = test_state_with_repo();
    let owner_id = test_owner_id();
    let member_id = "user_member".to_string();
    let (owner_secret, owner_token) = generate_git_clone_token(&owner_id).unwrap();
    let (member_secret, member_token) = generate_git_clone_token(&member_id).unwrap();
    {
        let mut repo = repo_with_readme();
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id.clone(),
            RepositoryMemberPermissions::default(),
        ));
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/secret.txt").unwrap(),
            ))
            .unwrap();
        repo.graph.commits[0].changes.push(FileChange {
            visibility: Visibility::Private,
            path: ScopePath::parse("/secret.txt").unwrap(),
            old_content: None,
            new_content: Some(source_blob("shared private view")),
        });
        repo.git_clone_tokens.extend([owner_token, member_token]);

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let mut owner_headers = HeaderMap::new();
    owner_headers.insert(
        AUTHORIZATION,
        format!("Basic {}", BASE64.encode(format!("scope:{owner_secret}")))
            .parse()
            .unwrap(),
    );
    let mut member_headers = HeaderMap::new();
    member_headers.insert(
        AUTHORIZATION,
        format!("Basic {}", BASE64.encode(format!("scope:{member_secret}")))
            .parse()
            .unwrap(),
    );

    let owner_projection =
        git_projection_for_request(&state, &owner_headers, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap();
    let member_projection =
        git_projection_for_request(&state, &member_headers, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap();

    assert_eq!(owner_projection.view_key, ProjectionViewKey::Private);
    assert_eq!(member_projection.view_key, ProjectionViewKey::Private);
    assert_eq!(
        owner_projection.commits[0].projected_id,
        member_projection.commits[0].projected_id
    );
    assert!(
        owner_projection.commits[0]
            .projected_id
            .starts_with("pv_private_")
    );
    assert_eq!(
        projection_cache_key(&owner_projection),
        projection_cache_key(&member_projection)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_clone_tokens_share_live_raw_snapshot_head() {
    let state = test_state_with_repo();
    let owner_id = test_owner_id();
    let member_id = "user_member".to_string();
    let (owner_secret, owner_token) = generate_git_clone_token(&owner_id).unwrap();
    let (member_secret, member_token) = generate_git_clone_token(&member_id).unwrap();
    let source = temp_git_repo("owner-upload-snapshot");
    fs::write(source.join("README.md"), "raw snapshot").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add readme").unwrap();
    commit_all(&source, "raw snapshot commit");
    let bare = std::env::temp_dir().join(format!(
        "scope-vcs-owner-upload-snapshot-bare-{}-{}",
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
        "clone snapshot bare repo",
    )
    .unwrap();
    let expected_head =
        git_stdout_text(&bare, &["rev-parse", DEFAULT_GIT_BRANCH], "snapshot head").unwrap();
    let pending =
        pending_import_from_staging_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME, &bare).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.pending_import = Some(pending);
        preview_publish_import(repo).unwrap();
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id.clone(),
            RepositoryMemberPermissions::default(),
        ));
        repo.git_clone_tokens.extend([owner_token, member_token]);
    }

    fs::write(source.join("README.md"), "staged snapshot").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add staged readme").unwrap();
    commit_all(&source, "staged snapshot commit");
    let staged_bare = std::env::temp_dir().join(format!(
        "scope-vcs-owner-upload-staged-snapshot-bare-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&staged_bare);
    run_git(
        None,
        &[
            "clone",
            "--bare",
            source.to_str().unwrap(),
            staged_bare.to_str().unwrap(),
        ],
        "clone staged snapshot bare repo",
    )
    .unwrap();
    let expected_staged_head = git_stdout_text(
        &staged_bare,
        &["rev-parse", DEFAULT_GIT_BRANCH],
        "staged snapshot head",
    )
    .unwrap();
    let update = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &staged_bare,
        &test_owner_id(),
    )
    .unwrap();
    let persisted = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        &test_owner_id(),
    )
    .unwrap();
    assert_eq!(persisted, PersistedReceivePackUpdate::Staged);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state_for_server = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state_for_server))
            .await
            .unwrap();
    });

    let owner_clone = std::env::temp_dir().join(format!(
        "scope-vcs-owner-private-clone-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let member_clone = std::env::temp_dir().join(format!(
        "scope-vcs-member-private-clone-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&owner_clone);
    let _ = fs::remove_dir_all(&member_clone);
    run_git(
        None,
        &[
            "clone",
            &format!("http://scope:{owner_secret}@{addr}/git/{TEST_REPO_ID}"),
            owner_clone.to_str().unwrap(),
        ],
        "clone live snapshot as owner",
    )
    .unwrap();
    run_git(
        None,
        &[
            "clone",
            &format!("http://scope:{member_secret}@{addr}/git/{TEST_REPO_ID}"),
            member_clone.to_str().unwrap(),
        ],
        "clone live snapshot as member",
    )
    .unwrap();
    let owner_head =
        git_stdout_text(&owner_clone, &["rev-parse", "HEAD"], "owner clone head").unwrap();
    let member_head =
        git_stdout_text(&member_clone, &["rev-parse", "HEAD"], "member clone head").unwrap();

    assert_eq!(owner_head, expected_head);
    assert_eq!(member_head, expected_head);
    assert_eq!(owner_head, member_head);
    assert_ne!(owner_head, expected_staged_head);

    server.abort();
    let _ = fs::remove_dir_all(&source);
    let _ = fs::remove_dir_all(&bare);
    let _ = fs::remove_dir_all(&staged_bare);
    let _ = fs::remove_dir_all(&owner_clone);
    let _ = fs::remove_dir_all(&member_clone);
}
