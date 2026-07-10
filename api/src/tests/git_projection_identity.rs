use super::*;
use crate::domain::store::{RepositoryAccess, RepositoryActor};

#[tokio::test]
async fn projection_view_key_treats_owner_as_private_view() {
    let access = RepositoryAccess {
        actor: RepositoryActor::Owner,
        can_read_private_files: false,
        can_push: false,
        can_change_file_visibility: false,
        can_apply_changes: false,
        can_manage_members: false,
        can_delete_repo: false,
    };

    assert_eq!(
        ProjectionViewKey::from_access(access),
        ProjectionViewKey::Private
    );
}

#[tokio::test]
async fn private_projection_cache_key_is_shared_by_owner_and_member() {
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
                ScopePath::parse("/secret.txt").unwrap(),
            ))
            .unwrap();
        repo.graph.commits[0].changes.push(FileChange {
            visibility: Visibility::Private,
            path: ScopePath::parse("/secret.txt").unwrap(),
            old_content: None,
            new_content: Some(source_blob("shared private view")),
        });
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let mut owner_headers = HeaderMap::new();
    owner_headers.insert(AUTHORIZATION, bearer_header().parse().unwrap());
    let mut member_headers = HeaderMap::new();
    member_headers.insert(
        AUTHORIZATION,
        bearer_header_for(member_subject, "member@example.com")
            .parse()
            .unwrap(),
    );

    let owner_projection = git_projection_for_request(
        &state,
        &owner_headers,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap();
    let member_projection = git_projection_for_request(
        &state,
        &member_headers,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
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
async fn permissioned_scope_sessions_share_raw_live_head() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let member_subject = "user_member";
    let member_id = crate::db::scope_user_id_for_auth_identity("clerk", member_subject);
    let source = temp_git_repo("owner-upload-snapshot");
    fs::write(source.join("README.md"), "raw snapshot").unwrap();
    run_git(Some(&source), &["add", "-A"], "add readme").unwrap();
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
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
    }
    apply_first_push_from_staging_repo(&state, &bare, repo_config(Visibility::Private)).await;
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .get_mut(TEST_REPO_ID)
            .unwrap()
            .members
            .push(test_repository_member(
                TEST_REPO_ID,
                member_id.clone(),
                RepositoryMemberPermissions::default(),
            ));
    }

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
    let remote = format!("http://{addr}/git/permissioned/{TEST_REPO_ID}");
    clone_with_bearer(
        &remote,
        &owner_clone,
        &bearer_header(),
        "clone live snapshot as owner",
    );
    clone_with_bearer(
        &remote,
        &member_clone,
        &bearer_header_for(member_subject, "member@example.com"),
        "clone live snapshot as member",
    );
    let owner_head =
        git_stdout_text(&owner_clone, &["rev-parse", "HEAD"], "owner clone head").unwrap();
    let member_head =
        git_stdout_text(&member_clone, &["rev-parse", "HEAD"], "member clone head").unwrap();

    assert_eq!(owner_head, expected_head);
    assert_eq!(member_head, expected_head);
    assert_eq!(owner_head, member_head);
    assert!(
        !owner_clone.join(".scope/repo.json").exists(),
        "raw Git clone should not receive private Scope config as tracked content"
    );
    assert!(
        !member_clone.join(".scope/repo.json").exists(),
        "raw Git clone should not receive private Scope config as tracked content"
    );

    server.abort();
    let _ = fs::remove_dir_all(&source);
    let _ = fs::remove_dir_all(&bare);
    let _ = fs::remove_dir_all(&owner_clone);
    let _ = fs::remove_dir_all(&member_clone);
}
