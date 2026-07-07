use super::*;
use crate::domain::requests::{
    GrantUserCreditsInput, Request, RequestActorRole, RequestBaseAudience, RequestState,
    SubmitRequestInput,
};

const PUBLIC_SUBJECT: &str = "user_public";
const PUBLIC_EMAIL: &str = "public@example.com";
const REQUEST_ID: &str = "req_1";
const REQUEST_REF: &str = "refs/scope/requests/req_1";
const PRIVATE_REQUEST_ID: &str = "req_private";
const PRIVATE_REQUEST_REF: &str = "refs/scope/requests/req_private";

#[tokio::test]
async fn request_author_receive_pack_does_not_require_push_intent() {
    let state = test_state_with_request();
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL)
            .parse()
            .unwrap(),
    );

    let access = receive_pack_access(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();

    assert!(matches!(
        access,
        ReceivePackAccess::RequestAuthor { author_id } if author_id == public_user_id()
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_git_request_ref_push_records_revision_without_touching_main() {
    let state = test_state_with_request();
    let state_for_server = state.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state_for_server))
            .await
            .unwrap();
    });

    let source = temp_checkout_dir("request-ref-push");
    let public_remote = format!("http://{addr}/git/public/{TEST_REPO_ID}");
    run_git(
        None,
        &["clone", &public_remote, source.to_str().unwrap()],
        "clone public repo for request",
    )
    .unwrap();
    fs::write(source.join("request.txt"), "request branch content\n").unwrap();
    run_git(Some(&source), &["add", "-A"], "add request changes").unwrap();
    commit_all(&source, "request change");
    configure_bearer_header(
        &source,
        &format!("http://{addr}/git/permissioned/{TEST_REPO_ID}"),
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
    );
    run_git(
        Some(&source),
        &[
            "push",
            &format!("http://{addr}/git/permissioned/{TEST_REPO_ID}"),
            &format!("HEAD:{REQUEST_REF}"),
        ],
        "push request ref",
    )
    .unwrap();
    let first_request_head = git_head_oid(&source);
    fs::write(source.join("request.txt"), "request branch content v2\n").unwrap();
    run_git(Some(&source), &["add", "-A"], "add second request change").unwrap();
    commit_all(&source, "request change v2");
    let request_head = git_head_oid(&source);
    run_git(
        Some(&source),
        &[
            "push",
            &format!("http://{addr}/git/permissioned/{TEST_REPO_ID}"),
            &format!("HEAD:{REQUEST_REF}"),
        ],
        "push request ref update",
    )
    .unwrap();

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert_eq!(
        repo.live_tree()
            .get(&ScopePath::parse("/README.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("hello")
    );
    assert!(
        !repo
            .live_tree()
            .contains_key(&ScopePath::parse("/request.txt").unwrap())
    );
    state
        .metadata
        .read(|catalog| {
            let request = catalog.requests.get(REQUEST_ID).unwrap();
            assert_eq!(request.head_oid, request_head);
            let snapshot = request.git_snapshot.as_ref().unwrap();
            source_blob_bytes(state.object_store.as_ref(), snapshot).unwrap();
            assert_eq!(catalog.request_events.len(), 3);
            Ok(())
        })
        .unwrap();
    let store_repo =
        crate::git::storage::request_ref_store_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let stored_head = git_stdout_text(&store_repo, &["rev-parse", REQUEST_REF], "read request ref")
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(stored_head, request_head);
    run_git(
        Some(&store_repo),
        &["update-ref", REQUEST_REF, &first_request_head],
        "simulate stale request ref cache",
    )
    .unwrap();
    let staging_repo = crate::git::request_refs::ensure_request_receive_pack_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &public_user_id(),
    )
    .unwrap();
    let restored_head = git_stdout_text(
        &staging_repo,
        &["rev-parse", REQUEST_REF],
        "read reconciled request ref",
    )
    .unwrap()
    .trim()
    .to_string();
    assert_eq!(restored_head, request_head);
    let _ = fs::remove_dir_all(staging_repo);
    fs::remove_dir_all(&store_repo).unwrap();
    let staging_repo = crate::git::request_refs::ensure_request_receive_pack_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &public_user_id(),
    )
    .unwrap();
    let restored_head = git_stdout_text(
        &staging_repo,
        &["rev-parse", REQUEST_REF],
        "read restored request ref",
    )
    .unwrap()
    .trim()
    .to_string();
    assert_eq!(restored_head, request_head);

    server.abort();
    let _ = fs::remove_dir_all(source);
    let _ = fs::remove_dir_all(staging_repo);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_ref_push_rejects_unsupported_tree_entries() {
    let state = test_state_with_request();
    let state_for_server = state.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state_for_server))
            .await
            .unwrap();
    });

    let source = temp_checkout_dir("request-ref-invalid-tree");
    let public_remote = format!("http://{addr}/git/public/{TEST_REPO_ID}");
    run_git(
        None,
        &["clone", &public_remote, source.to_str().unwrap()],
        "clone public repo for invalid request",
    )
    .unwrap();
    let commit = git_head_oid(&source);
    run_git(
        Some(&source),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{commit},vendor/submodule"),
        ],
        "add request gitlink",
    )
    .unwrap();
    commit_all(&source, "invalid request tree");
    let permissioned_remote = format!("http://{addr}/git/permissioned/{TEST_REPO_ID}");
    configure_bearer_header(
        &source,
        &permissioned_remote,
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
    );

    let output = run_git_output(
        Some(&source),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "push invalid request ref",
    )
    .unwrap();

    assert!(!output.status.success());
    state
        .metadata
        .read(|catalog| {
            let request = catalog.requests.get(REQUEST_ID).unwrap();
            assert_eq!(request.head_oid, "initial_request_head");
            assert!(request.git_snapshot.is_none());
            assert_eq!(catalog.request_events.len(), 1);
            Ok(())
        })
        .unwrap();

    server.abort();
    let _ = fs::remove_dir_all(source);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_request_author_cannot_push_main() {
    let state = test_state_with_request();
    let state_for_server = state.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state_for_server))
            .await
            .unwrap();
    });

    let source = temp_checkout_dir("request-main-rejected");
    let public_remote = format!("http://{addr}/git/public/{TEST_REPO_ID}");
    run_git(
        None,
        &["clone", &public_remote, source.to_str().unwrap()],
        "clone public repo for rejected main push",
    )
    .unwrap();
    fs::write(source.join("README.md"), "public main write\n").unwrap();
    run_git(Some(&source), &["add", "-A"], "add main change").unwrap();
    commit_all(&source, "try main");
    let permissioned_remote = format!("http://{addr}/git/permissioned/{TEST_REPO_ID}");
    configure_bearer_header(
        &source,
        &permissioned_remote,
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
    );

    let output = run_git_output(
        Some(&source),
        &["push", &permissioned_remote, "HEAD:main"],
        "push public main",
    )
    .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Scope request pushes only accept refs/scope/requests/*")
    );
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert_eq!(
        repo.live_tree()
            .get(&ScopePath::parse("/README.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("hello")
    );
    state
        .metadata
        .read(|catalog| {
            assert_eq!(
                catalog.requests.get(REQUEST_ID).unwrap().head_oid,
                "initial_request_head"
            );
            assert_eq!(catalog.request_events.len(), 1);
            Ok(())
        })
        .unwrap();

    server.abort();
    let _ = fs::remove_dir_all(source);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_actor_cannot_push_private_request_after_membership_loss() {
    let state = test_state_with_request();
    insert_private_request_for_public_user(&state);
    let state_for_server = state.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state_for_server))
            .await
            .unwrap();
    });

    let source = temp_checkout_dir("private-request-rejected");
    let public_remote = format!("http://{addr}/git/public/{TEST_REPO_ID}");
    run_git(
        None,
        &["clone", &public_remote, source.to_str().unwrap()],
        "clone public repo for private request push",
    )
    .unwrap();
    fs::write(
        source.join("private-request.txt"),
        "private request write\n",
    )
    .unwrap();
    run_git(Some(&source), &["add", "-A"], "add private request change").unwrap();
    commit_all(&source, "try private request");
    let permissioned_remote = format!("http://{addr}/git/permissioned/{TEST_REPO_ID}");
    configure_bearer_header(
        &source,
        &permissioned_remote,
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
    );

    let output = run_git_output(
        Some(&source),
        &[
            "push",
            &permissioned_remote,
            &format!("HEAD:{PRIVATE_REQUEST_REF}"),
        ],
        "push private request ref as public actor",
    )
    .unwrap();

    assert!(!output.status.success());
    state
        .metadata
        .read(|catalog| {
            assert_eq!(
                catalog.requests.get(PRIVATE_REQUEST_ID).unwrap().head_oid,
                "private_initial_head"
            );
            Ok(())
        })
        .unwrap();

    server.abort();
    let _ = fs::remove_dir_all(source);
}

fn test_state_with_request() -> AppState {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let public_user = UserAccount {
        id: public_user_id(),
        handle: "public".to_string(),
        email: PUBLIC_EMAIL.to_string(),
        email_verified: true,
    };
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.users.insert(public_user.id.clone(), public_user);
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
    }
    state
        .metadata
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "ledger_grant".to_string(),
            user_id: public_user_id(),
            amount_credits: 20,
            now_unix: 1,
        })
        .unwrap();
    state
        .metadata
        .submit_request(SubmitRequestInput {
            id: REQUEST_ID.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            author_user_id: public_user_id(),
            author_role: RequestActorRole::Public,
            base_audience: RequestBaseAudience::Public,
            target_branch: DEFAULT_GIT_BRANCH.to_string(),
            request_ref: REQUEST_REF.to_string(),
            base_main_oid: "base_main".to_string(),
            head_oid: "initial_request_head".to_string(),
            title: "Request branch".to_string(),
            stake_credits: 10,
            stake_ledger_entry_id: Some("ledger_stake".to_string()),
            event_id: "event_created".to_string(),
            now_unix: 2,
        })
        .unwrap();
    state
}

fn insert_private_request_for_public_user(state: &AppState) {
    state
        .metadata
        .update(|catalog| {
            catalog.requests.insert(
                PRIVATE_REQUEST_ID.to_string(),
                Request {
                    id: PRIVATE_REQUEST_ID.to_string(),
                    repo_id: TEST_REPO_ID.to_string(),
                    author_user_id: public_user_id(),
                    author_role: RequestActorRole::Member,
                    base_audience: RequestBaseAudience::Private,
                    target_branch: DEFAULT_GIT_BRANCH.to_string(),
                    request_ref: PRIVATE_REQUEST_REF.to_string(),
                    base_main_oid: "private_base_main".to_string(),
                    head_oid: "private_initial_head".to_string(),
                    git_snapshot: None,
                    title: "Former member request".to_string(),
                    state: RequestState::Submitted,
                    stake_credits: 0,
                    disposition: None,
                    settlement: None,
                    created_at_unix: 2,
                    updated_at_unix: 2,
                    resolved_at_unix: None,
                },
            );
            Ok(())
        })
        .unwrap();
}

fn configure_bearer_header(repo: &FsPath, remote: &str, bearer: &str) {
    run_git(
        Some(repo),
        &[
            "config",
            &format!("http.{remote}.extraHeader"),
            &format!("Authorization: {bearer}"),
        ],
        "configure bearer header",
    )
    .unwrap();
}

fn public_user_id() -> String {
    crate::db::scope_user_id_for_auth_identity("clerk", PUBLIC_SUBJECT)
}

fn temp_checkout_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "scope-vcs-{label}-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&path);
    path
}
