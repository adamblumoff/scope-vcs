use super::*;
use axum::body::Bytes;
use axum::http::header::CONTENT_ENCODING;
use flate2::{Compression, write::GzEncoder};
use std::{io::Write, process::Command, time::Duration};

const ZERO_OID: &str = "0000000000000000000000000000000000000000";

fn first_push_source(label: &str, content: &[u8]) -> TempGitRepo {
    let source = temp_git_repo(label);
    fs::write(source.join("README.md"), content).unwrap();
    run_git(Some(&source), &["add", "-A"], "add readme").unwrap();
    commit_all(&source, "initial");
    source
}

async fn receive_post(
    state: AppState,
    secret: &str,
    intent: String,
    body: Vec<u8>,
    gzip: bool,
) -> Response {
    let mut request = Request::builder()
        .method("POST")
        .uri("/git/permissioned/owner/repo/git-receive-pack")
        .header(CONTENT_TYPE, "application/x-git-receive-pack-request")
        .header("x-scope-push-intent", intent)
        .header(
            AUTHORIZATION,
            format!("Basic {}", BASE64.encode(format!("scope:{secret}"))),
        );
    if gzip {
        request = request.header(CONTENT_ENCODING, "gzip");
    }
    router(state)
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn receive_pack_accepts_gzip_encoded_request_body() {
    let (state, secret) = test_state_with_first_push_token().await;
    let source = first_push_source("gzip-first-push", b"hello over gzip receive-pack\n");
    let push_intent =
        create_test_push_intent(&state, &test_owner_id(), &git_head_oid(&source)).await;
    let response = receive_post(
        state.clone(),
        &secret,
        push_intent,
        gzip_bytes(&receive_pack_first_push_request(&source)),
        true,
    )
    .await;

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
            .map(|blob| blob_content(&state, blob))
            .as_deref(),
        Some("hello over gzip receive-pack\n")
    );
}

#[tokio::test]
async fn receive_pack_queues_orphan_objects_when_push_intent_does_not_match_head() {
    let (state, secret) = test_state_with_first_push_token().await;
    let readme = b"intent mismatch should be cleaned\n";
    let source = first_push_source("intent-head-mismatch-first-push", readme);
    let push_intent = create_test_push_intent(&state, &test_owner_id(), TEST_PUSH_HEAD_OID).await;
    let response = receive_post(
        state.clone(),
        &secret,
        push_intent,
        receive_pack_first_push_request(&source),
        false,
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(!state.test_object_store.contains_bytes(readme));
}

#[tokio::test]
async fn upload_pack_accepts_gzip_encoded_request_body() {
    let state = test_state_with_readme().await;
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
fn git_request_decoder_rejects_invalid_encoding_and_expansion() {
    for (encoding, body, limit, status, message) in [
        (
            "gzip",
            b"not gzip".to_vec(),
            1024,
            StatusCode::BAD_REQUEST,
            "invalid gzip Git request body",
        ),
        (
            "br",
            b"plain git body".to_vec(),
            1024,
            StatusCode::BAD_REQUEST,
            "unsupported Git content-encoding br",
        ),
        (
            "gzip",
            gzip_bytes(b"12345"),
            4,
            StatusCode::PAYLOAD_TOO_LARGE,
            "git request body is too large after decompression",
        ),
    ] {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_ENCODING, encoding.parse().unwrap());
        let error = decode_git_request_body(&headers, Bytes::from(body), limit).unwrap_err();
        assert_eq!(error.status(), status);
        assert!(error.message().contains(message));
    }
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
