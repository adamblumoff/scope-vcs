use super::*;

#[tokio::test]
async fn public_commit_history_omits_private_files_from_mixed_commits() {
    let state = test_state_with_repo();
    {
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.policy
            .add_rule(VisibilityRule::public(
                ScopePath::parse("/README.md").unwrap(),
            ))
            .unwrap();
        repo.graph.commits.push(LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: Vec::new(),
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "mixed first commit".to_string(),
            changes: vec![
                FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("hello")),
                },
                FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/secret.txt").unwrap(),
                    old_content: None,
                    new_content: Some(source_blob("secret")),
                },
            ],
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let list_response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/commits?audience=public")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body = response_json(list_response).await;
    assert_eq!(list_body["audience"], "public");
    assert_eq!(list_body["commits"].as_array().unwrap().len(), 1);
    assert_eq!(list_body["commits"][0]["change_count"], 1);
    let projected_id = list_body["commits"][0]["projected_id"]
        .as_str()
        .unwrap()
        .to_string();

    let detail_response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/repos/owner/repo/commits/{projected_id}?audience=public"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(detail_response.status(), StatusCode::OK);
    let detail_body = response_json(detail_response).await;
    assert_eq!(detail_body["files"].as_array().unwrap().len(), 1);
    assert_eq!(detail_body["files"][0]["path"], "/README.md");

    let secret_response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/repos/owner/repo/commits/{projected_id}/file-diff?audience=public&path=/secret.txt"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(secret_response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn public_commit_diff_does_not_leak_private_old_content() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = test_repo(&test_owner_id());
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.policy
            .add_rule(VisibilityRule::public(
                ScopePath::parse("/notes.md").unwrap(),
            ))
            .unwrap();
        let private_blob = source_blob("private draft");
        repo.graph.commits.push(LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: Vec::new(),
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "private draft".to_string(),
            changes: vec![FileChange {
                visibility: Visibility::Private,
                path: ScopePath::parse("/notes.md").unwrap(),
                old_content: None,
                new_content: Some(private_blob.clone()),
            }],
        });
        repo.graph.commits.push(LogicalCommit {
            id: "rv2".to_string(),
            parent_ids: vec!["rv1".to_string()],
            author_id: repo.record.owner_user_id.clone(),
            author_visibility: AuthorVisibility::Visible,
            message: "public release".to_string(),
            changes: vec![FileChange {
                visibility: Visibility::Public,
                path: ScopePath::parse("/notes.md").unwrap(),
                old_content: Some(private_blob),
                new_content: Some(source_blob("public release")),
            }],
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let public_list = router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/commits?audience=public")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(public_list.status(), StatusCode::OK);
    let public_body = response_json(public_list).await;
    assert_eq!(public_body["commits"].as_array().unwrap().len(), 1);
    let public_projected_id = public_body["commits"][0]["projected_id"].as_str().unwrap();

    let public_diff = router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/repos/owner/repo/commits/{public_projected_id}/file-diff?audience=public&path=/notes.md"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(public_diff.status(), StatusCode::OK);
    let public_diff_body = response_json(public_diff).await;
    assert_eq!(public_diff_body["kind"], "Added");
    assert_eq!(public_diff_body["old_content"], serde_json::Value::Null);
    assert_eq!(public_diff_body["new_content"], "public release");

    let owner_list = router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/commits?audience=owner")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(owner_list.status(), StatusCode::OK);
    let owner_body = response_json(owner_list).await;
    let owner_projected_id = owner_body["commits"][1]["projected_id"].as_str().unwrap();

    let owner_diff = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/repos/owner/repo/commits/{owner_projected_id}/file-diff?audience=owner&path=/notes.md"
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(owner_diff.status(), StatusCode::OK);
    let owner_diff_body = response_json(owner_diff).await;
    assert_eq!(owner_diff_body["kind"], "Modified");
    assert_eq!(owner_diff_body["old_content"], "private draft");
    assert_eq!(owner_diff_body["new_content"], "public release");
}
