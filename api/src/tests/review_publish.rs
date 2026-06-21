use super::*;
use crate::http::review::reject_staged_update_in_catalog;

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

    let private_files =
        files_for_visibility_update(&MemoryObjectStore::new(), &repo, &owner).unwrap();
    assert_eq!(private_files[0].visibility, Visibility::Private);

    repo.policy.add_rule(VisibilityRule::public(path)).unwrap();
    let public_files =
        files_for_visibility_update(&MemoryObjectStore::new(), &repo, &owner).unwrap();
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

#[tokio::test]
async fn publish_uses_verified_email_canonical_owner() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        repo.pending_import = Some(pending_import_fixture(vec![("README.md", "hello")]));
    }

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/publish")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("rotated-pairwise-owner", TEST_OWNER_EMAIL),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["publication_state"], "Published");
    assert_eq!(body["role"], "Owner");

    let catalog = lock_catalog(&state).unwrap();
    assert_eq!(catalog.users.len(), 1);
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::Published
    );
    assert!(repo.pending_import.is_none());
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

#[test]
fn rejecting_staged_update_deletes_unreferenced_bucket_objects() {
    let state = test_state_with_repo();
    let rejected_blob = source_blob("rejected private content");
    let rejected_key = rejected_blob.object_key.clone();
    {
        let mut repo = repo_with_readme();
        repo.staged_update = Some(StagedRepoUpdate {
            id: "staged_push_1".to_string(),
            branch: format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            base_live_commit_id: repo.graph.commits.last().map(|commit| commit.id.clone()),
            author_id: repo.record.owner_user_id.clone(),
            message: "reject me".to_string(),
            git_snapshot: source_blob("rejected staged git snapshot"),
            changes: vec![StagedFileChange {
                path: ScopePath::parse("/private.txt").unwrap(),
                old_content: None,
                new_content: Some(rejected_blob),
                visibility: Visibility::Private,
                kind: StagedFileChangeKind::Added,
            }],
        });
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    reject_staged_update_in_catalog(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();

    assert!(!MemoryObjectStore::new().contains_key(&rejected_key));
}
