use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_git_binary_and_crlf_round_trip_across_first_and_published_pushes() {
    let first = b"\x89PNG\r\n\x1a\nscope-binary-v1\0\x01";
    let updated = b"\x89PNG\r\n\x1a\nscope-binary-v2\0\x02\xff";
    let crlf = b"first line\r\ntrailing spaces   \r\nlast\t \r\n";
    let (state, first_secret) = test_state_with_first_push_token().await;
    let (origin, _server) = spawn_test_server(&state).await;
    let source = temp_git_repo("binary-round-trip");
    fs::write(source.join("image.png"), first).unwrap();
    fs::write(source.join("almost.txt"), crlf).unwrap();
    run_git(Some(&source), &["add", "-A"], "add byte-exact files").unwrap();
    commit_all(&source, "first binary push");

    let permissioned = |secret: &str| {
        format!("{origin}/git/permissioned/{TEST_REPO_ID}").replacen(
            "http://",
            &format!("http://scope:{secret}@"),
            1,
        )
    };
    let public = format!("{origin}/git/public/{TEST_REPO_ID}");
    let remote = permissioned(&first_secret);
    run_git(
        Some(&source),
        &["remote", "add", "scope", &remote],
        "add first push remote",
    )
    .unwrap();
    configure_push_intent_header(&state, &source, &remote, &test_owner_id()).await;
    run_git(
        Some(&source),
        &["push", "-u", "scope", "HEAD:main"],
        "push byte-exact first import",
    )
    .unwrap();

    let clone = unique_test_path("binary-first-clone");
    run_git(
        None,
        &["clone", &public, clone.to_str().unwrap()],
        "clone first projection",
    )
    .unwrap();
    assert_eq!(fs::read(clone.join("image.png")).unwrap(), first);
    assert_eq!(fs::read(clone.join("almost.txt")).unwrap(), crlf);

    cache_test_jwks(&state);
    let bearer = bearer_header();
    let remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}");
    run_git(
        Some(&source),
        &["remote", "set-url", "scope", &remote],
        "set push remote",
    )
    .unwrap();
    fs::write(source.join("image.png"), updated).unwrap();
    run_git(Some(&source), &["add", "-A"], "add binary update").unwrap();
    commit_all(&source, "update binary image");
    configure_push_intent_header(&state, &source, &remote, &test_owner_id()).await;
    run_git(
        Some(&source),
        &[
            "-c",
            &format!("http.{remote}.extraHeader=Authorization: {bearer}"),
            "push",
            "scope",
            "HEAD:main",
        ],
        "push binary update",
    )
    .unwrap();

    let final_clone = unique_test_path("binary-final-clone");
    run_git(
        None,
        &["clone", &public, final_clone.to_str().unwrap()],
        "clone updated projection",
    )
    .unwrap();
    assert_eq!(fs::read(final_clone.join("image.png")).unwrap(), updated);
    assert_eq!(fs::read(final_clone.join("almost.txt")).unwrap(), crlf);
}
