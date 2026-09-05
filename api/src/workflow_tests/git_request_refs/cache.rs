use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_view_cache_shares_authorized_output_across_users() {
    let state = test_state_with_request().await;
    insert_public_contributor(&state).await;
    state
        .metadata
        .requests()
        .mutate_request_for_tests(REQUEST_ID, |request| {
            request.submitted_at_unix = Some(3);
            request.updated_at_unix = 3;
        })
        .await
        .unwrap();
    let author = authorization_headers(bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL));
    let contributor =
        authorization_headers(bearer_header_for(CONTRIBUTOR_SUBJECT, CONTRIBUTOR_EMAIL));
    let read = |headers| {
        git_upload_pack_repo_for_request(
            &state,
            headers,
            TEST_REPO_OWNER,
            TEST_REPO_NAME,
            GitRemoteMode::Permissioned,
        )
    };
    let (first, second) = tokio::join!(read(&author), read(&contributor));
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.as_ref(), second.as_ref());
    let anonymous = git_upload_pack_repo_for_request(
        &state,
        &HeaderMap::new(),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Public,
    )
    .await
    .unwrap();
    assert_eq!(first.as_ref(), anonymous.as_ref());
    assert_eq!(
        fs::read_dir(state.repository_engine.cache_root())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("read-view-"))
            .count(),
        1
    );
    let marker = first.as_ref().join("cache-reuse-proof");
    fs::write(&marker, "original build").unwrap();
    let reused = read(&contributor).await.unwrap();
    assert_eq!(first.as_ref(), reused.as_ref());
    assert_eq!(fs::read_to_string(marker).unwrap(), "original build");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_view_cache_keeps_draft_visibility_separate() {
    let state = test_state_with_request().await;
    insert_public_contributor(&state).await;
    let author = authorization_headers(bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL));
    let contributor =
        authorization_headers(bearer_header_for(CONTRIBUTOR_SUBJECT, CONTRIBUTOR_EMAIL));
    let author_view = git_upload_pack_repo_for_request(
        &state,
        &author,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap();
    let contributor_view = git_upload_pack_repo_for_request(
        &state,
        &contributor,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap();
    assert_ne!(author_view.as_ref(), contributor_view.as_ref());
    assert!(
        git_stdout_text(
            author_view.as_ref(),
            &["rev-parse", "--verify", REQUEST_REF],
            "author draft ref"
        )
        .is_ok()
    );
    assert!(
        git_stdout_text(
            contributor_view.as_ref(),
            &["rev-parse", "--verify", REQUEST_REF],
            "unrelated draft ref"
        )
        .is_err()
    );
}
