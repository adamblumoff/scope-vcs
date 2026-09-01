use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_commit_metadata_remains_identifiable_and_inspectable() {
    let state = test_state_with_request().await;
    let (source, permissioned_remote, _server, _) =
        request_checkout(&state, "request-ref-oversized-metadata").await;
    fs::write(source.join("oversized.txt"), "inspectable content\n").unwrap();
    run_git(
        Some(&source),
        &["add", "-A"],
        "stage oversized metadata commit",
    )
    .unwrap();
    let message_path = source.join(".git/oversized-message");
    fs::write(&message_path, vec![b'x'; 70 * 1024]).unwrap();
    run_git(
        Some(&source),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "commit",
            "-F",
            message_path.to_str().unwrap(),
        ],
        "commit oversized request metadata",
    )
    .unwrap();
    run_git(
        Some(&source),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "push oversized request metadata",
    )
    .unwrap();

    let commit_oid = git_head_oid(&source);
    let app = router(state);
    let revisions = public_get_json(
        &app,
        format!("/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{REQUEST_ID}/changes"),
    )
    .await;
    let latest = revisions["revisions"].as_array().unwrap().last().unwrap();
    assert_eq!(revisions["review_revision_id"], latest["id"]);
    assert_eq!(latest["inspection"], "Incomplete");
    assert_eq!(latest["commits"][0]["oid"], commit_oid);
    assert_eq!(latest["commits"][0]["author"], serde_json::Value::Null);
    assert_eq!(latest["commits"][0]["message"], "");
    assert_eq!(latest["commits"][0]["files"][0]["path"], "oversized.txt");
    assert_eq!(latest["commits"][0]["files_truncated"], false);

    let missing_revision = app
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{REQUEST_ID}/changes?revision=missing"
                ))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_revision.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_revision_stays_open_and_publishes_refresh() {
    let state = test_state_with_request().await;
    let (source, permissioned_remote, _server, _first_request_head) =
        request_checkout(&state, "request-ref-revision-rollback").await;
    state
        .metadata
        .requests()
        .submit_request(SubmitRequestInput {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            actor_is_author: false,
            actor_can_submit: false,
            event_id: "event_submit_for_revision".to_string(),
            now_unix: 4,
        })
        .await
        .unwrap();
    let before_event_count = request_event_count(&state).await;
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);

    push_change(
        &source,
        &permissioned_remote,
        REQUEST_REF,
        "request.txt",
        "request branch content after review invalidation\n",
        "revision invalidates review",
    )
    .unwrap();

    let after = stored_request(&state, REQUEST_ID).await;
    assert_eq!(after.state(), RequestState::Open);
    assert_eq!(after.head_oid, git_head_oid(&source));
    assert_eq!(request_event_count(&state).await, before_event_count + 1);
    let request_events = state
        .metadata
        .requests()
        .request_events_for_tests()
        .await
        .unwrap();
    assert!(request_events.iter().any(|event| {
        event.request_id == REQUEST_ID && event.kind == RequestEventKind::RevisionPushed
    }));
    let store_repo =
        crate::git::storage::request_ref_store_repo_path(&state, &test_repo_incarnation());
    let stored_head = git_stdout_text(
        &store_repo,
        &["rev-parse", REQUEST_REF],
        "read invalidating request ref",
    )
    .unwrap();
    assert_eq!(stored_head.trim(), after.head_oid);
    assert!(events.try_recv().is_ok());
}
