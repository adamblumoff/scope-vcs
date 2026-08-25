use super::*;
use serde_json::json;

async fn mutate_repo(state: &AppState, configure: impl FnOnce(&mut Repository)) {
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, configure)
        .await
        .unwrap();
}

fn commit(
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

fn change(
    visibility: Visibility,
    path: &str,
    old: Option<scope_domain::content::SourceBlob>,
    new: Option<scope_domain::content::SourceBlob>,
) -> FileChange {
    FileChange {
        visibility,
        path: ScopePath::parse(path).unwrap(),
        old_content: old,
        new_content: new,
    }
}

fn set_private(repo: &mut Repository, public_path: Option<&str>) {
    repo.repo_config = repo_config(Visibility::Private);
    repo.policy = Policy::new(Visibility::Private);
    if let Some(path) = public_path {
        repo.policy
            .add_rule(VisibilityRule::public(ScopePath::parse(path).unwrap()))
            .unwrap();
    }
}

fn add_mixed_commit(state: &AppState, repo: &mut Repository) {
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
    let rebuilt = state
        .metadata
        .jobs()
        .run_ready_outbox_jobs(
            "repo-visibility-test",
            10,
            &|| {
                crate::persistence::unix_now()
                    .map_err(crate::error::ApiError::into_operator_diagnostic)
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
    assert_eq!(rebuilt.failed, 0);
    assert!(rebuilt.completed > 0);

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
    assert_eq!(body["size_bytes"], "public readme".len());
    assert_eq!(body["content"]["kind"], "text");
    assert_eq!(body["content"]["text"], "public readme");
}

#[tokio::test]
async fn file_content_falls_back_to_the_domain_while_projection_rebuilds() {
    let state = test_state_with_repo();
    mutate_repo(&state, |repo| {
        repo.graph.commits.push(commit(
            "rv1",
            None,
            "public version",
            vec![change(
                Visibility::Public,
                "/README.md",
                None,
                Some(source_blob(&state, "public readme")),
            )],
        ));
    })
    .await;

    let response = get(
        state,
        "/v1/repos/owner/repo/files/content?path=README.md",
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await["content"]["text"],
        "public readme"
    );
}

#[tokio::test]
async fn public_file_content_uses_visible_domain_state_while_projection_rebuilds() {
    let state = test_state_with_repo();
    mutate_repo(&state, |repo| {
        set_private(repo, Some("/README.md"));
        add_mixed_commit(&state, repo);
    })
    .await;

    let response = get(
        state,
        "/v1/repos/owner/repo/files/content?path=README.md",
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["content"]["text"], "hello");
}

#[tokio::test]
async fn file_content_hides_unpublished_repo_during_projection_rebuild() {
    let state = test_state_with_repo();
    mutate_repo(&state, |repo| {
        repo.record.lifecycle_state = RepoLifecycleState::AwaitingFirstPush;
        repo.graph.commits.push(commit(
            "rv1",
            None,
            "private version",
            vec![change(
                Visibility::Private,
                "/secret.txt",
                None,
                Some(source_blob(&state, "secret")),
            )],
        ));
    })
    .await;

    let response = get(
        state,
        "/v1/repos/owner/repo/files/content?path=secret.txt",
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
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
async fn canonical_rules_alone_do_not_publish_a_repository() {
    let state = test_state_with_repo();
    mutate_repo(&state, |repo| {
        set_private(repo, Some("/.scope/RULES.md"));
        repo.graph.commits.push(commit(
            "rv1",
            None,
            "initial",
            vec![
                change(
                    Visibility::Public,
                    "/.scope/RULES.md",
                    None,
                    Some(source_blob(&state, "")),
                ),
                change(
                    Visibility::Private,
                    "/secret.txt",
                    None,
                    Some(source_blob(&state, "secret")),
                ),
            ],
        ));
    })
    .await;
    state
        .metadata
        .jobs()
        .run_ready_outbox_jobs(
            "rules-only-visibility-test",
            10,
            &|| {
                crate::persistence::unix_now()
                    .map_err(crate::error::ApiError::into_operator_diagnostic)
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
    let files = get(state.clone(), "/v1/repos/owner/repo/files", None).await;
    assert_eq!(files.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        get(
            state,
            "/v1/repos/owner/repo/files/content?path=secret.txt",
            None,
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn logged_in_non_member_cannot_read_repo_without_public_project_files() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let auth = bearer_header_for("user_other", "other@example.com");
    let repo = get(state.clone(), "/v1/repos/owner/repo", Some(&auth)).await;
    assert_eq!(repo.status(), StatusCode::NOT_FOUND);

    let files = get(state, "/v1/repos/owner/repo/files", Some(&auth)).await;
    assert_eq!(files.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn owner_profile_lists_only_repositories_visible_to_the_viewer() {
    let state = test_state_with_repo();

    let anonymous = get(state.clone(), "/v1/users/owner/repos", None).await;
    assert_eq!(anonymous.status(), StatusCode::OK);
    assert_eq!(response_json(anonymous).await["repositories"], json!([]));

    cache_test_jwks(&state);
    let owner = get(
        state.clone(),
        "/v1/users/owner/repos",
        Some(&bearer_header()),
    )
    .await;
    assert_eq!(owner.status(), StatusCode::OK);
    assert_eq!(
        response_json(owner).await["repositories"][0]["id"],
        TEST_REPO_ID
    );

    mutate_repo(&state, |repo| add_mixed_commit(&state, repo)).await;
    state
        .metadata
        .jobs()
        .run_ready_outbox_jobs(
            "owner-profile-test",
            10,
            &|| {
                crate::persistence::unix_now()
                    .map_err(crate::error::ApiError::into_operator_diagnostic)
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();

    let anonymous = get(state.clone(), "/v1/users/owner/repos", None).await;
    assert_eq!(anonymous.status(), StatusCode::OK);
    let profile = response_json(anonymous).await;
    assert_eq!(profile["handle"], "owner");
    assert_eq!(profile["repositories"][0]["id"], TEST_REPO_ID);
    assert!(
        profile["repositories"][0]
            .get("default_visibility")
            .is_none()
    );

    let unknown = get(state, "/v1/users/missing/repos", None).await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
}
