use super::*;

fn history_repo(commits: Vec<LogicalCommit>, public_path: Option<&str>) -> StoredRepository {
    let mut repo = test_repo(&test_owner_id());
    repo.record.default_visibility = Visibility::Private;
    repo.policy = Policy::new(Visibility::Private);
    if let Some(path) = public_path {
        repo.policy
            .add_rule(VisibilityRule::public(ScopePath::parse(path).unwrap()))
            .unwrap();
    }
    repo.graph.commits = commits;
    repo
}

fn history_commit(
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

fn history_change(
    path: &str,
    visibility: Visibility,
    old: Option<crate::domain::store::SourceBlob>,
    new: Option<crate::domain::store::SourceBlob>,
) -> FileChange {
    FileChange {
        path: ScopePath::parse(path).unwrap(),
        visibility,
        old_content: old,
        new_content: new,
    }
}

async fn history_get(state: AppState, uri: impl AsRef<str>, private: bool) -> Response {
    let mut request = Request::builder().method("GET").uri(uri.as_ref());
    if private {
        request = request.header(AUTHORIZATION, bearer_header());
    }
    router(state)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn first_projected_id(state: AppState, audience: &str) -> String {
    let response = history_get(
        state,
        format!("/v1/repos/owner/repo/commits?audience={audience}"),
        audience == "private",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await["commits"][0]["projected_id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn public_commit_diff_does_not_leak_private_old_content() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let private = source_blob(&state, "private draft");
    replace_test_repo(
        &state,
        history_repo(
            vec![
                history_commit(
                    "rv1",
                    None,
                    "private draft",
                    vec![history_change(
                        "/notes.md",
                        Visibility::Private,
                        None,
                        Some(private.clone()),
                    )],
                ),
                history_commit(
                    "rv2",
                    Some("rv1"),
                    "public release",
                    vec![history_change(
                        "/notes.md",
                        Visibility::Public,
                        Some(private),
                        Some(source_blob(&state, "public release")),
                    )],
                ),
            ],
            Some("/notes.md"),
        ),
    )
    .await;

    let public_id = first_projected_id(state.clone(), "public").await;
    let detail = history_get(
        state.clone(),
        format!("/v1/repos/owner/repo/commits/{public_id}?audience=public"),
        false,
    )
    .await;
    assert_eq!(detail.status(), StatusCode::OK);
    assert_eq!(response_json(detail).await["files"][0]["path"], "/notes.md");
    let public = history_get(
        state.clone(),
        format!(
            "/v1/repos/owner/repo/commits/{public_id}/file-diff?audience=public&path=/notes.md"
        ),
        false,
    )
    .await;
    assert_eq!(public.status(), StatusCode::OK);
    let public = response_json(public).await;
    assert_eq!(public["kind"], "Added");
    assert_eq!(public["old_content"], serde_json::Value::Null);
    assert_text_content(&public["new_content"], "public release");

    let private_list = history_get(
        state.clone(),
        "/v1/repos/owner/repo/commits?audience=private",
        true,
    )
    .await;
    let private_id = response_json(private_list).await["commits"][1]["projected_id"]
        .as_str()
        .unwrap()
        .to_string();
    let private = history_get(
        state,
        format!(
            "/v1/repos/owner/repo/commits/{private_id}/file-diff?audience=private&path=/notes.md"
        ),
        true,
    )
    .await;
    assert_eq!(private.status(), StatusCode::OK);
    let private = response_json(private).await;
    assert_eq!(private["kind"], "Modified");
    assert_text_content(&private["old_content"], "private draft");
    assert_text_content(&private["new_content"], "public release");
}

#[tokio::test]
async fn public_commit_history_generation_tracks_visible_history() {
    let state = test_state_with_repo();
    let first = source_blob(&state, "first");
    replace_test_repo(
        &state,
        history_repo(
            vec![history_commit(
                "rv1",
                None,
                "first",
                vec![history_change(
                    "/README.md",
                    Visibility::Public,
                    None,
                    Some(first.clone()),
                )],
            )],
            Some("/README.md"),
        ),
    )
    .await;

    let response = history_get(
        state.clone(),
        "/v1/repos/owner/repo/commits?audience=public",
        false,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let first_generation = response_json(response).await["generation"]
        .as_str()
        .unwrap()
        .to_string();

    replace_test_repo(
        &state,
        history_repo(
            vec![
                history_commit(
                    "rv1",
                    None,
                    "first",
                    vec![history_change(
                        "/README.md",
                        Visibility::Public,
                        None,
                        Some(first.clone()),
                    )],
                ),
                history_commit(
                    "rv2",
                    Some("rv1"),
                    "second",
                    vec![history_change(
                        "/README.md",
                        Visibility::Public,
                        Some(first),
                        Some(source_blob(&state, "second")),
                    )],
                ),
            ],
            Some("/README.md"),
        ),
    )
    .await;

    let response = history_get(state, "/v1/repos/owner/repo/commits?audience=public", false).await;
    assert_eq!(response.status(), StatusCode::OK);
    let second_generation = response_json(response).await["generation"]
        .as_str()
        .unwrap()
        .to_string();

    assert_eq!(first_generation.len(), 64);
    assert_ne!(first_generation, second_generation);
}
