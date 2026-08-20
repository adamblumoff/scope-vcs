use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn merge_route_persists_git_content_once() {
    let (state, owner_source) = test_state_with_mergeable_request().await;
    insert_member_user(&state).await;
    let (source, remote, _server, first_request_head) =
        request_checkout(&state, "request-http-merge").await;
    configure_bearer_header(
        &owner_source,
        &remote,
        &bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
    );
    fs::write(owner_source.join("README.md"), "upstream public change\n").unwrap();
    run_git(
        Some(&owner_source),
        &["add", "README.md"],
        "stage public main advance",
    )
    .unwrap();
    commit_all(&owner_source, "advance public main");
    configure_push_intent_header(&state, &owner_source, &remote, &test_owner_id()).await;
    run_git(
        Some(&owner_source),
        &[
            "push",
            &remote,
            &format!("HEAD:refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "advance public main",
    )
    .unwrap();
    let public_remote = remote.replace("/git/permissioned/", "/git/public/");
    run_git(
        Some(&source),
        &[
            "fetch",
            &public_remote,
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "fetch updated public main",
    )
    .unwrap();
    run_git(
        Some(&source),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "merge",
            "--no-ff",
            "--no-commit",
            "FETCH_HEAD",
        ],
        "merge updated public main into request",
    )
    .unwrap();
    commit_all(&source, "update request onto public main");
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "README.md",
        "request resolution\n",
        "resolve request against public main",
    )
    .unwrap();
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "request-second.txt",
        "second request commit\n",
        "second request change",
    )
    .unwrap();
    let request_head = git_head_oid(&source);
    let app = router(state.clone());

    let submitted = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/submit"
                ))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(submitted.status(), StatusCode::OK);

    let merge_request = || {
        axum::http::Request::builder()
            .method("POST")
            .uri(format!(
                "/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/merge"
            ))
            .header(
                AUTHORIZATION,
                bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
            )
            .body(Body::empty())
            .unwrap()
    };
    let merged = app.clone().oneshot(merge_request()).await.unwrap();
    let merged_status = merged.status();
    let merged = response_json(merged).await;
    assert_eq!(merged_status, StatusCode::OK, "{merged}");
    assert_eq!(merged["request"]["state"], "Merged");
    assert_eq!(merged["request"]["merged_head_oid"], request_head);
    assert_ne!(merged["request"]["merged_main_oid"], request_head);
    assert_eq!(
        merged["request"]["mergeability"]["current_main_oid"],
        merged["request"]["merged_main_oid"]
    );
    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("request resolution\n")
    );
    assert_eq!(
        live_file_content(&state, "/request.txt").await.as_deref(),
        Some("request branch content\n")
    );
    assert_eq!(
        live_file_content(&state, "/request-second.txt")
            .await
            .as_deref(),
        Some("second request commit\n")
    );

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let public_projection = project_graph(
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Public,
    );
    let public_repo = projection_bare_repo_for_state(
        &state,
        &repo.record.id,
        &public_projection,
        repo.git_head.as_ref(),
        &repo.git_pack_spans,
    )
    .unwrap();
    assert_eq!(
        git_stdout_text(
            &public_repo,
            &["rev-parse", &format!("refs/heads/{DEFAULT_GIT_BRANCH}")],
            "read public main after request merge",
        )
        .unwrap()
        .trim(),
        request_head
    );
    run_git(
        Some(&public_repo),
        &[
            "merge-base",
            "--is-ancestor",
            &first_request_head,
            &request_head,
        ],
        "verify first contributor commit remains public ancestry",
    )
    .unwrap();
    run_git(
        Some(&public_repo),
        &["fsck", "--full"],
        "verify materialized public repository",
    )
    .unwrap();
    let public_history = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/commits?audience=public"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_history.status(), StatusCode::NOT_IMPLEMENTED);
    let public_preview = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/projection-preview?audience=public"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_preview.status(), StatusCode::NOT_IMPLEMENTED);
    let committed_repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    match &committed_repo.graph.commits.last().unwrap().origin {
        LogicalCommitOrigin::PublicRequestMerge {
            request_id,
            request_head_oid,
            commits,
            ..
        } => {
            assert_eq!(request_id, REQUEST_ID);
            assert_eq!(request_head_oid, &request_head);
            assert_eq!(
                commits.last().map(|commit| &commit.oid),
                Some(&request_head)
            );
        }
        origin => panic!("expected public request merge origin, got {origin:?}"),
    }
    let replay = app.oneshot(merge_request()).await.unwrap();
    assert_eq!(replay.status(), StatusCode::CONFLICT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_merge_rejects_path_made_private_after_request_push() {
    let state = test_state_with_mergeable_owner_public_request().await;
    insert_member_user(&state).await;
    let (source, remote, _server) = request_push_checkout(
        &state,
        "maintainer-public-policy-merge",
        TEST_CLERK_USER_ID,
        TEST_OWNER_EMAIL,
    )
    .await;
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "request.txt",
        "request branch content\n",
        "request change",
    )
    .unwrap();
    let app = router(state.clone());

    let submitted = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/submit"
                ))
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let submitted_status = submitted.status();
    let submitted = response_json(submitted).await;
    assert_eq!(submitted_status, StatusCode::OK, "{submitted}");

    let private_path = ScopePath::parse("/request.txt").unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.repo_config.visibility.rules.push(
                scope_domain::repo_config::RepoConfigVisibilityRule {
                    path: private_path.as_str().to_string(),
                    visibility: ConfigVisibility::Private,
                },
            );
            repo.bump_change_version();
        })
        .await
        .unwrap();

    let merged = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/merge"
                ))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let merged_status = merged.status();
    let merged = response_json(merged).await;
    assert_eq!(merged_status, StatusCode::CONFLICT, "{merged}");
    assert_eq!(live_file_content(&state, "/request.txt").await, None);
    assert_eq!(
        stored_request(&state, REQUEST_ID).await.state(),
        RequestState::Open
    );
}
