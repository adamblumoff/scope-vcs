use super::*;
use crate::domain::requests::{
    RecordWorkingRequestUploadInput, RequestActorRole, RequestAudience, StartRequestInput,
    SubmitRequestInput, canonical_request_ref,
};

#[tokio::test]
async fn reads_the_uploaded_request_ref_bundle() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let main_repo = temp_git_repo("request-review-main");
    fs::write(main_repo.join("README.md"), "hello\n").unwrap();
    fs::write(main_repo.join("script.sh"), "#!/bin/sh\necho hello\n").unwrap();
    run_git(Some(&main_repo), &["add", "."], "stage review main").unwrap();
    commit_all(&main_repo, "initial");
    let main_oid = git_head_oid(&main_repo);
    let main_git = git_segment_manifest_from_repo(&state, TEST_REPO_ID, &main_repo, None)
        .await
        .unwrap();

    let request_repo = temp_git_repo("request-review-branch");
    run_git(
        Some(&request_repo),
        &["pull", main_repo.to_str().unwrap(), "main"],
        "seed review request branch",
    )
    .unwrap();
    fs::write(request_repo.join("README.md"), "hello from request\n").unwrap();
    run_git(Some(&request_repo), &["add", "."], "stage review request").unwrap();
    run_git(
        Some(&request_repo),
        &["update-index", "--chmod=+x", "script.sh"],
        "make review script executable",
    )
    .unwrap();
    commit_all(&request_repo, "request change");
    let request_head = git_head_oid(&request_repo);
    let request_ref = canonical_request_ref("review");
    run_git(
        Some(&request_repo),
        &["update-ref", &request_ref, &request_head],
        "create review request ref",
    )
    .unwrap();
    let request_snapshot =
        git_snapshot_from_ref(&state, TEST_REPO_ID, &request_repo, &request_ref).unwrap();

    let mut repo = repo_with_readme(&state);
    repo.git_head = Some(main_git.head);
    repo.git_segments.push(main_git.segment);
    replace_test_repo(&state, repo).await;
    state
        .metadata
        .start_request(StartRequestInput {
            id: "req_review".to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: "review".to_string(),
            author_user_id: test_owner_id(),
            title: Some("Review request".to_string()),
            author_role: RequestActorRole::Owner,
            audience: RequestAudience::Private,
            base_main_oid: main_oid,
            event_id: "event_req_review_started".to_string(),
            now_unix: 2,
        })
        .await
        .unwrap();
    state
        .metadata
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: "req_review".to_string(),
            actor_user_id: test_owner_id(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: request_head.clone(),
            git_snapshot: request_snapshot,
            now_unix: 3,
        })
        .await
        .unwrap();
    state
        .metadata
        .submit_request(SubmitRequestInput {
            request_id: "req_review".to_string(),
            actor_user_id: test_owner_id(),
            expected_head_oid: request_head,
            stake_credits: 0,
            stake_ledger_entry_id: None,
            event_id: "event_req_review_created".to_string(),
            now_unix: 4,
        })
        .await
        .unwrap();

    let app = router(state);
    let changes = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/repos/owner/repo/requests/req_review/changes")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(changes.status(), StatusCode::OK);
    let changes = response_json(changes).await;
    assert_eq!(changes["files"][0]["path"], "README.md");
    assert_eq!(changes["files"][0]["kind"], "Modified");
    assert_eq!(changes["files"][1]["path"], "script.sh");
    assert_eq!(changes["files"][1]["old_mode"], "100644");
    assert_eq!(changes["files"][1]["new_mode"], "100755");

    let diff = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/repos/owner/repo/requests/req_review/file-diff?path=README.md")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(diff.status(), StatusCode::OK);
    let diff = response_json(diff).await;
    assert_text_content(&diff["old_content"], "hello\n");
    assert_text_content(&diff["new_content"], "hello from request\n");

    let mode_diff = app
        .oneshot(
            Request::builder()
                .uri("/v1/repos/owner/repo/requests/req_review/file-diff?path=script.sh")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mode_diff.status(), StatusCode::OK);
    let mode_diff = response_json(mode_diff).await;
    assert_eq!(mode_diff["old_mode"], "100644");
    assert_eq!(mode_diff["new_mode"], "100755");
    assert_text_content(&mode_diff["old_content"], "#!/bin/sh\necho hello\n");
    assert_text_content(&mode_diff["new_content"], "#!/bin/sh\necho hello\n");
}
