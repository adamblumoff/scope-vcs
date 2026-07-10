use super::*;

#[tokio::test]
async fn permissioned_git_projection_accepts_scope_session_for_repo_member() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let member_clerk_id = "user_member";
    let member_id = crate::db::scope_user_id_for_auth_identity("clerk", member_clerk_id);
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
        repo.graph.commits[0].changes.push(FileChange {
            visibility: Visibility::Private,
            path: ScopePath::parse("/member-secret.txt").unwrap(),
            old_content: None,
            new_content: Some(source_blob("member can read")),
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.users.insert(
            member_id.clone(),
            test_user(member_id, "member", "member@example.com"),
        );
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        bearer_header_for(member_clerk_id, "member@example.com")
            .parse()
            .unwrap(),
    );

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

    assert_eq!(projection.view_key, ProjectionViewKey::Private);
    assert!(visible_paths.contains(&"/README.md"));
    assert!(visible_paths.contains(&"/member-secret.txt"));
}

#[tokio::test]
async fn permissioned_git_projection_accepts_basic_scope_cli_session_for_repo_owner() {
    let state = test_state_with_repo();
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

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let owner = UserAccount {
        id: test_owner_id(),
        handle: TEST_REPO_OWNER.to_string(),
        email: TEST_OWNER_EMAIL.to_string(),
        email_verified: true,
    };
    let grant = state
        .metadata
        .create_cli_exchange_grant(&owner)
        .await
        .unwrap();
    let cli_session = state
        .metadata
        .exchange_cli_grant(&grant.exchange_token)
        .await
        .unwrap()
        .session_token;
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!("Basic {}", BASE64.encode(format!("scope:{cli_session}")))
            .parse()
            .unwrap(),
    );

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

    assert_eq!(projection.view_key, ProjectionViewKey::Private);
    assert!(visible_paths.contains(&"/README.md"));
    assert!(visible_paths.contains(&"/secret.txt"));
}

#[tokio::test]
async fn permissioned_git_projection_rejects_scope_session_without_target_repo_membership() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let other_clerk_id = "user_other_owner";
    let other_id = crate::db::scope_user_id_for_auth_identity("clerk", other_clerk_id);
    {
        let mut other_repo = test_repo(&other_id);
        other_repo.record.id = "other/owned".to_string();
        other_repo.record.owner_handle = "other".to_string();
        other_repo.record.name = "owned".to_string();
        other_repo.graph.repo_id = other_repo.record.id.clone();
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.users.insert(
            other_id.clone(),
            test_user(other_id, "other", "other@example.com"),
        );
        catalog
            .repositories
            .insert(other_repo.record.id.clone(), other_repo);
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        bearer_header_for(other_clerk_id, "other@example.com")
            .parse()
            .unwrap(),
    );

    let error = git_projection_for_request(
        &state,
        &headers,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap_err();

    assert_eq!(error.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn public_git_projection_ignores_credentials_and_omits_private_files() {
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
            new_content: Some(source_blob("owner only")),
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, bearer_header().parse().unwrap());

    let projection = git_projection_for_request(
        &state,
        &headers,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Public,
    )
    .await
    .unwrap();
    let visible_paths = projection
        .commits
        .iter()
        .flat_map(|commit| &commit.changes)
        .map(|change| change.path.as_str())
        .collect::<Vec<_>>();

    assert_eq!(projection.view_key, ProjectionViewKey::Public);
    assert!(visible_paths.contains(&"/README.md"));
    assert!(!visible_paths.contains(&"/owner-secret.txt"));
}
