use super::*;

#[tokio::test]
async fn pending_publish_repo_session_is_owner_only() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingPublish;
    }
    let app = router(state);

    let public_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(public_response.status(), StatusCode::NOT_FOUND);

    let owner_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/session")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(owner_response.status(), StatusCode::OK);
    let body = response_json(owner_response).await;
    assert_eq!(body["principal_id"], test_owner_id());
    assert_eq!(body["capabilities"]["read"], true);
}

#[test]
fn pending_import_review_uses_default_visibility() {
    let mut repo = test_repo(&test_owner_id());
    repo.record.publication_state = RepoPublicationState::PendingPublish;
    repo.record.default_visibility = Visibility::Private;
    repo.policy = Policy::new(Visibility::Private, repo.record.owner_user_id.clone());
    repo.pending_import = Some(pending_import_fixture(vec![
        ("README.md", "hello"),
        ("src/main.rs", "fn main() {}"),
    ]));
    let owner = Principal {
        id: repo.record.owner_user_id.clone(),
        kind: PrincipalKind::User,
    };

    let files = pending_import_files(&repo, &owner).unwrap();

    assert_eq!(files.len(), 2);
    assert!(
        files
            .iter()
            .all(|file| file.visibility == Visibility::Private)
    );
}

#[test]
fn pending_visibility_toggles_apply_before_publish() {
    let mut repo = test_repo(&test_owner_id());
    repo.record.publication_state = RepoPublicationState::PendingPublish;
    repo.pending_import = Some(pending_import_fixture(vec![("README.md", "hello")]));
    let path = ScopePath::parse("/README.md").unwrap();
    repo.policy
        .add_rule(VisibilityRule::private(path.clone(), repo_owner_ids(&repo)))
        .unwrap();
    let owner = Principal {
        id: repo.record.owner_user_id.clone(),
        kind: PrincipalKind::User,
    };

    let private_files = files_for_visibility_update(&repo, &owner).unwrap();
    assert_eq!(private_files[0].visibility, Visibility::Private);

    repo.policy.add_rule(VisibilityRule::public(path)).unwrap();
    let public_files = files_for_visibility_update(&repo, &owner).unwrap();
    assert_eq!(public_files[0].visibility, Visibility::Public);
}

#[test]
fn zero_file_publish_promotes_pending_import() {
    let mut repo = test_repo(&test_owner_id());
    repo.record.publication_state = RepoPublicationState::PendingPublish;
    repo.pending_import = Some(pending_import_fixture(Vec::new()));

    promote_pending_import(&mut repo).unwrap();

    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::Published
    );
    assert!(repo.pending_import.is_none());
    assert_eq!(repo.graph.commits.len(), 1);
    assert!(repo.graph.commits[0].changes.is_empty());
}

#[test]
fn publish_is_one_time() {
    let mut repo = test_repo(&test_owner_id());
    repo.record.publication_state = RepoPublicationState::PendingPublish;
    repo.pending_import = Some(pending_import_fixture(vec![("README.md", "hello")]));

    promote_pending_import(&mut repo).unwrap();
    let error = promote_pending_import(&mut repo).unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
}

#[test]
fn repo_settings_review_pushes_default_on() {
    assert!(RepoSettings::default().review_pushes_before_applying);
    assert!(!RepoSettings::default().include_ignored_files);
}
