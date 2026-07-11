use super::*;

async fn mutate_repo(state: &AppState, configure: impl FnOnce(&mut StoredRepository)) {
    state
        .metadata
        .mutate_repository_for_tests(TEST_REPO_ID, configure)
        .await
        .unwrap();
}

fn commit(
    id: &str,
    parent: Option<&str>,
    message: &str,
    changes: Vec<FileChange>,
) -> LogicalCommit {
    LogicalCommit {
        id: id.into(),
        parent_ids: parent.into_iter().map(str::to_string).collect(),
        author_id: test_owner_id(),
        author_visibility: AuthorVisibility::Visible,
        message: message.into(),
        changes,
    }
}

fn change(
    visibility: Visibility,
    path: &str,
    old: Option<crate::domain::store::SourceBlob>,
    new: Option<crate::domain::store::SourceBlob>,
) -> FileChange {
    FileChange {
        visibility,
        path: ScopePath::parse(path).unwrap(),
        old_content: old,
        new_content: new,
    }
}

fn set_private(repo: &mut StoredRepository, public_path: Option<&str>) {
    repo.record.default_visibility = Visibility::Private;
    repo.policy = Policy::new(Visibility::Private);
    if let Some(path) = public_path {
        repo.policy
            .add_rule(VisibilityRule::public(ScopePath::parse(path).unwrap()))
            .unwrap();
    }
}

fn add_mixed_commit(state: &AppState, repo: &mut StoredRepository) {
    repo.graph.commits.push(commit(
        "rv1",
        None,
        "initial",
        vec![
            change(
                Visibility::Public,
                "/README.md",
                None,
                Some(source_blob(state, "hello")),
            ),
            change(
                Visibility::Private,
                "/secret.txt",
                None,
                Some(source_blob(state, "secret")),
            ),
        ],
    ));
}

async fn get(state: AppState, uri: &str, authorization: Option<&str>) -> Response {
    let mut request = Request::builder().method("GET").uri(uri);
    if let Some(authorization) = authorization {
        request = request.header(AUTHORIZATION, authorization);
    }
    router(state)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn public_files_use_the_projected_blob() {
    let state = test_state_with_repo();
    mutate_repo(&state, |repo| {
        let public = source_blob(&state, "public readme");
        repo.graph.commits.extend([
            commit(
                "rv1",
                None,
                "public version",
                vec![change(
                    Visibility::Public,
                    "/README.md",
                    None,
                    Some(public.clone()),
                )],
            ),
            commit(
                "rv2",
                Some("rv1"),
                "private draft",
                vec![change(
                    Visibility::Private,
                    "/README.md",
                    Some(public),
                    Some(source_blob(&state, "private draft")),
                )],
            ),
        ]);
    })
    .await;

    let files = get(state.clone(), "/v1/repos/owner/repo/files", None).await;
    assert_eq!(files.status(), StatusCode::OK);
    assert_eq!(response_json(files).await[0]["path"], "/README.md");
    let response = get(
        state,
        "/v1/repos/owner/repo/files/content?path=README.md",
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["path"], "/README.md");
    assert_eq!(body["content"]["kind"], "text");
    assert_eq!(body["content"]["text"], "public readme");
}

#[tokio::test]
async fn file_content_rejects_empty_path() {
    let response = get(
        test_state_with_repo(),
        "/v1/repos/owner/repo/files/content?path=",
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn published_repo_projection_preview_serves_public_file_subset() {
    let state = test_state_with_repo();
    mutate_repo(&state, |repo| {
        set_private(repo, Some("/README.md"));
        add_mixed_commit(&state, repo);
        repo.graph.commits.push(commit(
            "rv2",
            Some("rv1"),
            "private notes",
            vec![change(
                Visibility::Private,
                "/notes/private.md",
                None,
                Some(source_blob(&state, "private notes")),
            )],
        ));
    })
    .await;
    cache_test_jwks(&state);

    let public = get(
        state.clone(),
        "/v1/repos/owner/repo/projection-preview?audience=public",
        None,
    )
    .await;
    assert_eq!(public.status(), StatusCode::OK);
    let public = response_json(public).await;
    assert_eq!(public["audience"], "public");
    assert_eq!(public["source"], "live");
    assert_eq!(public["summary"]["visible_files"], 1);
    assert_eq!(public["summary"]["hidden_files"], 0);
    assert_eq!(public["summary"]["hidden_commits"], 0);
    assert_eq!(public["files"][0]["path"], "/README.md");

    let owner = get(
        state,
        "/v1/repos/owner/repo/projection-preview?audience=public",
        Some(&bearer_header()),
    )
    .await;
    assert_eq!(owner.status(), StatusCode::OK);
    let owner = response_json(owner).await;
    assert_eq!(owner["summary"]["visible_files"], 1);
    assert_eq!(owner["summary"]["hidden_files"], 2);
    assert_eq!(owner["summary"]["hidden_commits"], 1);
}

#[tokio::test]
async fn published_default_private_repo_without_public_files_stays_hidden() {
    let state = test_state_with_repo();
    mutate_repo(&state, |repo| {
        set_private(repo, None);
        repo.graph.commits.push(commit(
            "rv1",
            None,
            "initial",
            vec![change(
                Visibility::Private,
                "/secret.txt",
                None,
                Some(source_blob(&state, "secret")),
            )],
        ));
    })
    .await;
    assert_eq!(
        get(state, "/v1/repos/owner/repo/files", None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn logged_in_non_member_reads_empty_public_repo_as_public() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let auth = bearer_header_for("user_other", "other@example.com");
    let repo = get(state.clone(), "/v1/repos/owner/repo", Some(&auth)).await;
    assert_eq!(repo.status(), StatusCode::OK);
    let repo = response_json(repo).await;
    assert_eq!(repo["id"], TEST_REPO_ID);
    assert_eq!(repo["access"]["actor"], "Public");
    assert_eq!(repo["change_version"], 0);

    let files = get(state, "/v1/repos/owner/repo/files", Some(&auth)).await;
    assert_eq!(files.status(), StatusCode::OK);
    assert!(response_json(files).await.as_array().unwrap().is_empty());
}
