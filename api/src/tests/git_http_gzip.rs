use super::*;
use axum::body::Bytes;
use axum::http::header::CONTENT_ENCODING;
use flate2::{Compression, write::GzEncoder};
use std::{io::Write, process::Command, time::Duration};

const ZERO_OID: &str = "0000000000000000000000000000000000000000";

#[tokio::test]
async fn receive_pack_accepts_gzip_encoded_request_body() {
    let state = test_state_with_repo();
    let (secret, token) = generate_first_push_token(&test_owner_id()).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.first_push_token = Some(token);
    }
    let source = temp_git_repo("gzip-first-push");
    fs::write(source.join("README.md"), "hello over gzip receive-pack\n").unwrap();
    run_git(Some(&source), &["add", "-A"], "add readme").unwrap();
    commit_all(&source, "initial");
    let push_intent =
        create_test_push_intent(&state, &test_owner_id(), &git_head_oid(&source)).await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/git/permissioned/owner/repo/git-receive-pack")
                .header(CONTENT_TYPE, "application/x-git-receive-pack-request")
                .header(CONTENT_ENCODING, "gzip")
                .header("x-scope-push-intent", push_intent)
                .header(
                    AUTHORIZATION,
                    format!("Basic {}", BASE64.encode(format!("scope:{secret}"))),
                )
                .body(Body::from(gzip_bytes(&receive_pack_first_push_request(
                    &source,
                ))))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert!(
        body.windows(b"unpack ok".len())
            .any(|window| window == b"unpack ok")
    );

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::Published
    );
    assert!(repo.first_push_token.is_none());
    assert_eq!(
        repo.live_tree()
            .get(&ScopePath::parse("/README.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("hello over gzip receive-pack\n")
    );

    let _ = fs::remove_dir_all(source);
}

#[tokio::test]
async fn receive_pack_cleans_uploaded_blobs_when_push_intent_does_not_match_head() {
    let state = test_state_with_repo();
    let (secret, token) = generate_first_push_token(&test_owner_id()).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.first_push_token = Some(token);
    }
    let source = temp_git_repo("intent-head-mismatch-first-push");
    let readme = b"intent mismatch should be cleaned\n";
    fs::write(source.join("README.md"), readme).unwrap();
    run_git(Some(&source), &["add", "-A"], "add readme").unwrap();
    commit_all(&source, "initial");
    let push_intent = create_test_push_intent(&state, &test_owner_id(), TEST_PUSH_HEAD_OID).await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/git/permissioned/owner/repo/git-receive-pack")
                .header(CONTENT_TYPE, "application/x-git-receive-pack-request")
                .header("x-scope-push-intent", push_intent)
                .header(
                    AUTHORIZATION,
                    format!("Basic {}", BASE64.encode(format!("scope:{secret}"))),
                )
                .body(Body::from(receive_pack_first_push_request(&source)))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(!MemoryObjectStore::new().contains_bytes(readme));
    let rx_root = state.git_cache_root().unwrap().join("git-rx");
    if rx_root.exists() {
        assert!(fs::read_dir(rx_root).unwrap().next().is_none());
    }

    let _ = fs::remove_dir_all(source);
}

#[tokio::test]
async fn upload_pack_accepts_gzip_encoded_request_body() {
    let state = test_state_with_repo();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
    }
    let repo_path = git_upload_pack_repo_for_request(
        &state,
        &HeaderMap::new(),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Public,
    )
    .await
    .unwrap();
    let head = git_stdout_text(
        &repo_path,
        &["rev-parse", DEFAULT_GIT_BRANCH],
        "upload head",
    )
    .unwrap();
    let request = upload_pack_want_request(head.trim());
    let gzipped_request = gzip_bytes(&request);

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/git/public/owner/repo/git-upload-pack")
                .header(CONTENT_TYPE, "application/x-git-upload-pack-request")
                .header(CONTENT_ENCODING, "gzip")
                .body(Body::from(gzipped_request))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert!(body.starts_with(b"0008NAK\nPACK"));
}

#[test]
fn git_request_decoder_rejects_invalid_gzip_body() {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_ENCODING, "gzip".parse().unwrap());

    let error =
        decode_git_request_body(&headers, Bytes::from_static(b"not gzip"), 1024).unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert!(error.message.contains("invalid gzip Git request body"));
}

#[test]
fn git_request_decoder_rejects_unsupported_content_encoding() {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_ENCODING, "br".parse().unwrap());

    let error =
        decode_git_request_body(&headers, Bytes::from_static(b"plain git body"), 1024).unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert_eq!(error.message, "unsupported Git content-encoding br");
}

#[test]
fn git_request_decoder_limits_decompressed_body_size() {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_ENCODING, "gzip".parse().unwrap());

    let error =
        decode_git_request_body(&headers, Bytes::from(gzip_bytes(b"12345")), 4).unwrap_err();

    assert_eq!(error.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        error.message,
        "git request body is too large after decompression"
    );
}

fn upload_pack_want_request(oid: &str) -> Vec<u8> {
    let want = format!("want {oid}\n");
    let mut request = pkt_line(want.as_bytes());
    request.extend_from_slice(b"0000");
    request.extend_from_slice(&pkt_line(b"done\n"));
    request
}

fn receive_pack_first_push_request(source: &FsPath) -> Vec<u8> {
    let head = git_stdout_text(source, &["rev-parse", "HEAD"], "source head").unwrap();
    let mut pack_command = Command::new("git");
    pack_command
        .arg("-C")
        .arg(source)
        .arg("pack-objects")
        .arg("--revs")
        .arg("--stdout");
    let pack = git_command_output_with_timeout(
        &mut pack_command,
        Some(b"HEAD\n".to_vec()),
        Duration::from_secs(5),
    )
    .unwrap();

    let command = format!(
        "{ZERO_OID} {} refs/heads/{DEFAULT_GIT_BRANCH}\0 report-status side-band-64k object-format=sha1\n",
        head.trim()
    );
    let mut request = pkt_line(command.as_bytes());
    request.extend_from_slice(b"0000");
    request.extend(pack);
    request
}

fn gzip_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}
