use super::*;

fn history_repo(commits: Vec<LogicalCommit>, public_path: Option<&str>) -> StoredRepository {
    let mut repo = test_repo(&test_owner_id());
    repo.repo_config = repo_config(Visibility::Private);
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
    _parent: Option<&str>,
    message: &str,
    changes: Vec<FileChange>,
) -> LogicalCommit {
    LogicalCommit {
        id: id.into(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: id.to_string(),
        },
        author_id: test_owner_id(),
        message: message.into(),
        changes,
    }
}

fn history_change(
    path: &str,
    visibility: Visibility,
    old: Option<scope_domain::store::SourceBlob>,
    new: Option<scope_domain::store::SourceBlob>,
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
        format!("/v1/repos/owner/repo/history?audience={audience}"),
        audience == "private",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await["entries"][0]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn history_defaults_to_the_readers_broadest_audience() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    replace_test_repo(&state, paged_history_repo(&state, 1)).await;

    let public = history_get(state.clone(), "/v1/repos/owner/repo/history", false).await;
    assert_eq!(public.status(), StatusCode::OK);
    assert_eq!(response_json(public).await["audience"], "public");

    let maintainer = history_get(state, "/v1/repos/owner/repo/history", true).await;
    assert_eq!(maintainer.status(), StatusCode::OK);
    assert_eq!(response_json(maintainer).await["audience"], "private");
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
        format!("/v1/repos/owner/repo/history/{public_id}?audience=public"),
        false,
    )
    .await;
    assert_eq!(detail.status(), StatusCode::OK);
    assert_eq!(response_json(detail).await["files"][0]["path"], "/notes.md");
    let public = history_get(
        state.clone(),
        format!(
            "/v1/repos/owner/repo/history/{public_id}/file-diff?audience=public&path=/notes.md"
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
        "/v1/repos/owner/repo/history?audience=private",
        true,
    )
    .await;
    let private_id = response_json(private_list).await["entries"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let private = history_get(
        state,
        format!(
            "/v1/repos/owner/repo/history/{private_id}/file-diff?audience=private&path=/notes.md"
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
async fn public_history_generation_tracks_visible_history() {
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
        "/v1/repos/owner/repo/history?audience=public",
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

    let response = history_get(state, "/v1/repos/owner/repo/history?audience=public", false).await;
    assert_eq!(response.status(), StatusCode::OK);
    let second_generation = response_json(response).await["generation"]
        .as_str()
        .unwrap()
        .to_string();

    assert_eq!(first_generation.len(), 64);
    assert_ne!(first_generation, second_generation);
}

fn paged_history_repo(state: &AppState, count: usize) -> StoredRepository {
    let mut previous = None;
    let commits = (1..=count)
        .map(|index| {
            let next = source_blob(state, &format!("version {index}"));
            let commit = history_commit(
                &format!("rv{index}"),
                None,
                &format!("push {index}"),
                vec![history_change(
                    "/README.md",
                    Visibility::Public,
                    previous.take(),
                    Some(next.clone()),
                )],
            );
            previous = Some(next);
            commit
        })
        .collect();
    history_repo(commits, Some("/README.md"))
}

#[tokio::test]
async fn history_pages_are_newest_first_and_exhaust_cleanly() {
    let state = test_state_with_repo();
    replace_test_repo(&state, paged_history_repo(&state, 55)).await;

    let first = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?audience=public",
        false,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_json(first).await;
    let first_entries = first["entries"].as_array().unwrap();
    assert_eq!(first_entries.len(), 50);
    assert_eq!(first_entries[0]["source_id"], "rv55");
    assert_eq!(first_entries[49]["source_id"], "rv6");
    let cursor = first["next_cursor"].as_str().unwrap();

    replace_test_repo(&state, paged_history_repo(&state, 56)).await;
    let second = history_get(
        state,
        format!("/v1/repos/owner/repo/history?audience=public&before={cursor}"),
        false,
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second = response_json(second).await;
    let second_entries = second["entries"].as_array().unwrap();
    assert_eq!(second_entries.len(), 5);
    assert_eq!(second_entries[0]["source_id"], "rv5");
    assert_eq!(second_entries[4]["source_id"], "rv1");
    assert!(second["next_cursor"].is_null());
}

#[tokio::test]
async fn history_cursor_rejects_invalid_values_and_other_audiences() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    replace_test_repo(&state, paged_history_repo(&state, 51)).await;

    let invalid = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?audience=public&before=not-a-cursor",
        false,
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let first = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?audience=public",
        false,
    )
    .await;
    let first = response_json(first).await;
    let cursor = first["next_cursor"].as_str().unwrap();
    let wrong_audience = history_get(
        state.clone(),
        format!("/v1/repos/owner/repo/history?audience=private&before={cursor}"),
        true,
    )
    .await;
    assert_eq!(wrong_audience.status(), StatusCode::BAD_REQUEST);

    replace_test_repo(&state, paged_history_repo(&state, 1)).await;
    let missing_boundary = history_get(
        state,
        format!("/v1/repos/owner/repo/history?audience=public&before={cursor}"),
        false,
    )
    .await;
    assert_eq!(missing_boundary.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn history_entries_report_their_update_kind() {
    let state = test_state_with_repo();
    let first = source_blob(&state, "first");
    let second = source_blob(&state, "second");
    let mut repo = history_repo(
        vec![
            history_commit(
                "rv1",
                None,
                "push",
                vec![history_change(
                    "/README.md",
                    Visibility::Public,
                    None,
                    Some(first.clone()),
                )],
            ),
            LogicalCommit {
                id: "rv2".into(),
                origin: LogicalCommitOrigin::PrivateRequestMerge {
                    request_id: "request-1".into(),
                    request_head_oid: "head-1".into(),
                },
                author_id: test_owner_id(),
                message: "merged request".into(),
                changes: vec![history_change(
                    "/README.md",
                    Visibility::Public,
                    Some(first),
                    Some(second.clone()),
                )],
            },
        ],
        Some("/README.md"),
    );
    repo.visibility_events
        .push(scope_domain::projection::VisibilityEvent {
            id: "visibility-1".into(),
            after_commit_id: Some("rv2".into()),
            source_commit_id: None,
            author_id: test_owner_id(),
            path: ScopePath::parse("/README.md").unwrap(),
            old_visibility: Visibility::Public,
            new_visibility: Visibility::Private,
            current_content: Some(second),
        });
    replace_test_repo(&state, repo).await;
    cache_test_jwks(&state);

    let response = history_get(state, "/v1/repos/owner/repo/history?audience=public", true).await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = response_json(response).await;
    let entries = response["entries"].as_array().unwrap();
    assert_eq!(entries[0]["kind"], "visibility_change");
    assert_eq!(entries[1]["kind"], "merged_request");
    assert_eq!(entries[2]["kind"], "push");
}
