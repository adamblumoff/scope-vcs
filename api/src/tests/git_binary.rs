use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_git_binary_first_push_round_trips_through_clone() {
    let image = b"\x89PNG\r\n\x1a\nscope-binary-v1\0\xff";
    let clone =
        first_push_publish_clone_round_trip("binary-first-push", &[("image.png", image)]).await;

    assert_eq!(fs::read(clone.join("image.png")).unwrap(), image);
    let _ = fs::remove_dir_all(clone);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_git_almost_text_crlf_round_trips_without_normalization() {
    let almost_text = b"first line\r\ntrailing spaces   \r\nlast\t \r\n";
    let clone = first_push_publish_clone_round_trip(
        "almost-text-first-push",
        &[("almost.txt", almost_text)],
    )
    .await;

    assert_eq!(fs::read(clone.join("almost.txt")).unwrap(), almost_text);
    let _ = fs::remove_dir_all(clone);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_git_binary_published_push_round_trips_through_clone() {
    let state = test_state_with_repo();
    let secret = "scope_git_binary_update";
    let original = b"\x89PNG\r\n\x1a\nscope-binary-v1\0\x01";
    let updated = b"\x89PNG\r\n\x1a\nscope-binary-v2\0\x02\xff";
    {
        let mut repo = repo_with_readme();
        repo.settings.review_pushes_before_applying = false;
        repo.git_push_token = Some(GitPushToken {
            token_hash: git_push_token_hash(secret),
            owner_user_id: repo.record.owner_user_id.clone(),
            created_at_unix: unix_now(),
        });
        repo.graph.commits[0].changes.push(FileChange {
            visibility: Visibility::Public,
            path: ScopePath::parse("/image.png").unwrap(),
            old_content: None,
            new_content: Some(source_blob_from_bytes(original)),
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let (addr, server) = start_git_http_server(state.clone()).await;
    let source = temp_path("binary-published-update-source");
    let remote = format!("http://scope:{secret}@{addr}/git/permissioned/{TEST_REPO_ID}");
    let public_remote = format!("http://{addr}/git/public/{TEST_REPO_ID}");
    run_git(
        None,
        &["clone", &public_remote, source.to_str().unwrap()],
        "clone published binary repo",
    )
    .unwrap();
    run_git(
        Some(&source),
        &["remote", "set-url", "origin", &remote],
        "point origin at permissioned Scope remote",
    )
    .unwrap();
    assert_eq!(fs::read(source.join("image.png")).unwrap(), original);

    fs::write(source.join("image.png"), updated).unwrap();
    run_git(Some(&source), &["add", "-A"], "add binary update").unwrap();
    commit_all(&source, "update binary image");
    configure_push_intent_header(&state, &source, &remote, &test_owner_id());
    run_git(
        Some(&source),
        &["push", "origin", "HEAD:main"],
        "push binary update over http",
    )
    .unwrap();

    let public_clone = temp_path("binary-published-update-public-clone");
    run_git(
        None,
        &["clone", &public_remote, public_clone.to_str().unwrap()],
        "clone public binary projection",
    )
    .unwrap();

    assert_eq!(fs::read(public_clone.join("image.png")).unwrap(), updated);

    server.abort();
    let _ = fs::remove_dir_all(source);
    let _ = fs::remove_dir_all(public_clone);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_git_projection_omits_private_binary_file() {
    let state = test_state_with_repo();
    {
        let mut repo = repo_with_readme();
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/secret.png").unwrap(),
            ))
            .unwrap();
        repo.graph.commits[0].changes.push(FileChange {
            visibility: Visibility::Private,
            path: ScopePath::parse("/secret.png").unwrap(),
            old_content: None,
            new_content: Some(source_blob_from_bytes(b"\x89PNG\r\n\x1a\nprivate\0\xff")),
        });

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let (addr, server) = start_git_http_server(state).await;
    let public_clone = temp_path("private-binary-public-clone");
    let public_remote = format!("http://{addr}/git/public/{TEST_REPO_ID}");
    run_git(
        None,
        &["clone", &public_remote, public_clone.to_str().unwrap()],
        "clone public projection without private binary",
    )
    .unwrap();

    assert!(public_clone.join("README.md").is_file());
    assert!(!public_clone.join("secret.png").exists());

    server.abort();
    let _ = fs::remove_dir_all(public_clone);
}

async fn first_push_publish_clone_round_trip(label: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let state = test_state_with_repo();
    let (secret, state_for_server) = {
        let (secret, token) = generate_first_push_token(&test_owner_id()).unwrap();
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.first_push_token = Some(token);
        (secret, state.clone())
    };
    let (addr, server) = start_git_http_server(state_for_server).await;

    let source = temp_git_repo(label);
    for (path, bytes) in files {
        let path = source.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }
    run_git(Some(&source), &["add", "-A"], "add round-trip files").unwrap();
    commit_all(&source, "round-trip files");

    let remote = format!("http://scope:{secret}@{addr}/git/permissioned/{TEST_REPO_ID}");
    run_git(
        Some(&source),
        &["remote", "add", "scope", &remote],
        "add scope remote",
    )
    .unwrap();
    configure_push_intent_header(&state, &source, &remote, &test_owner_id());
    run_git(
        Some(&source),
        &["push", "-u", "scope", "HEAD:main"],
        "push first import over http",
    )
    .unwrap();

    let clone = temp_path(&format!("{label}-public-clone"));
    let public_remote = format!("http://{addr}/git/public/{TEST_REPO_ID}");
    run_git(
        None,
        &["clone", &public_remote, clone.to_str().unwrap()],
        "clone public projection",
    )
    .unwrap();

    server.abort();
    let _ = fs::remove_dir_all(source);
    clone
}

async fn start_git_http_server(state: AppState) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    (addr.to_string(), server)
}

fn temp_path(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "scope-vcs-{label}-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&path);
    path
}
