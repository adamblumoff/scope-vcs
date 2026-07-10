use super::*;
use crate::domain::requests::{
    AddRequestEditorInput, GrantUserCreditsInput, Request, RequestActorRole, RequestBaseAudience,
    RequestState, StartRequestInput, SubmitRequestInput,
};

const PUBLIC_SUBJECT: &str = "user_public";
const PUBLIC_EMAIL: &str = "public@example.com";
const EDITOR_SUBJECT: &str = "user_editor";
const EDITOR_EMAIL: &str = "editor@example.com";
const STRANGER_SUBJECT: &str = "user_stranger";
const STRANGER_EMAIL: &str = "stranger@example.com";
const MEMBER_SUBJECT: &str = "user_member";
const MEMBER_EMAIL: &str = "member@example.com";
const REQUEST_ID: &str = "req_1";
const REQUEST_REF: &str = "refs/scope/requests/req_1";
const PRIVATE_REQUEST_ID: &str = "req_private";
const PRIVATE_REQUEST_REF: &str = "refs/scope/requests/req_private";

mod privacy;

#[tokio::test]
async fn request_editor_receive_pack_does_not_require_push_intent() {
    let state = test_state_with_request();
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL)
            .parse()
            .unwrap(),
    );
    headers.insert(
        "x-scope-push-intent",
        "stale-request-push-intent".parse().unwrap(),
    );

    let access = receive_pack_access(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();

    assert!(matches!(
        access,
        ReceivePackAccess::RequestEditor { author_id } if author_id == public_user_id()
    ));
}

#[tokio::test]
async fn request_editor_receive_pack_requires_current_repo_read() {
    let state = test_state_with_request();
    state
        .metadata
        .update(|catalog| {
            let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
            repo.policy = Policy::new(Visibility::Private);
            repo.graph.commits.clear();
            Ok(())
        })
        .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL)
            .parse()
            .unwrap(),
    );

    let error = receive_pack_access(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap_err();

    assert_eq!(error.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn invited_public_editor_receive_pack_does_not_require_push_intent() {
    let state = test_state_with_request();
    insert_editor_user(&state);
    invite_editor_to_request(&state);
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        bearer_header_for(EDITOR_SUBJECT, EDITOR_EMAIL)
            .parse()
            .unwrap(),
    );

    let access = receive_pack_access(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();

    assert!(matches!(
        access,
        ReceivePackAccess::RequestEditor { author_id } if author_id == editor_user_id()
    ));
}

#[tokio::test]
async fn uninvited_public_user_cannot_receive_pack_for_request_refs() {
    let state = test_state_with_request();
    insert_stranger_user(&state);
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        bearer_header_for(STRANGER_SUBJECT, STRANGER_EMAIL)
            .parse()
            .unwrap(),
    );

    let error = receive_pack_access(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap_err();

    assert_eq!(error.status, StatusCode::NOT_FOUND);
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
    state
        .metadata
        .submit_request(SubmitRequestInput {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            expected_head_oid: first_request_head.clone(),
            stake_credits: 10,
            stake_ledger_entry_id: Some("ledger_stake".to_string()),
            event_id: "event_created".to_string(),
            now_unix: 4,
        })
        .unwrap();
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
            assert_eq!(catalog.request_events.len(), 2);
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
async fn invited_public_editor_can_push_request_ref() {
    let state = test_state_with_request();
    insert_editor_user(&state);
    invite_editor_to_request(&state);
    let state_for_server = state.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state_for_server))
            .await
            .unwrap();
    });

    let source = temp_checkout_dir("request-ref-editor-push");
    let public_remote = format!("http://{addr}/git/public/{TEST_REPO_ID}");
    run_git(
        None,
        &["clone", &public_remote, source.to_str().unwrap()],
        "clone public repo for editor request",
    )
    .unwrap();
    fs::write(source.join("editor.txt"), "editor request branch content\n").unwrap();
    run_git(Some(&source), &["add", "-A"], "add editor request changes").unwrap();
    commit_all(&source, "editor request change");
    let permissioned_remote = format!("http://{addr}/git/permissioned/{TEST_REPO_ID}");
    configure_bearer_header(
        &source,
        &permissioned_remote,
        &bearer_header_for(EDITOR_SUBJECT, EDITOR_EMAIL),
    );

    run_git(
        Some(&source),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "push request ref as invited editor",
    )
    .unwrap();

    let request_head = git_head_oid(&source);
    state
        .metadata
        .read(|catalog| {
            let request = catalog.requests.get(REQUEST_ID).unwrap();
            assert_eq!(request.head_oid, request_head);
            assert!(request.git_snapshot.is_some());
            assert!(catalog.request_events.is_empty());
            Ok(())
        })
        .unwrap();

    server.abort();
    let _ = fs::remove_dir_all(source);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maintainer_can_push_request_ref_without_being_author_or_editor() {
    let state = test_state_with_request();
    insert_member_user(&state);
    let state_for_server = state.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state_for_server))
            .await
            .unwrap();
    });

    let source = temp_checkout_dir("request-ref-maintainer-push");
    let public_remote = format!("http://{addr}/git/public/{TEST_REPO_ID}");
    run_git(
        None,
        &["clone", &public_remote, source.to_str().unwrap()],
        "clone public repo for maintainer request",
    )
    .unwrap();
    fs::write(
        source.join("maintainer.txt"),
        "maintainer request branch content\n",
    )
    .unwrap();
    run_git(
        Some(&source),
        &["add", "-A"],
        "add maintainer request changes",
    )
    .unwrap();
    commit_all(&source, "maintainer request change");
    let permissioned_remote = format!("http://{addr}/git/permissioned/{TEST_REPO_ID}");
    configure_bearer_header(
        &source,
        &permissioned_remote,
        &bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
    );

    run_git(
        Some(&source),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "push request ref as maintainer",
    )
    .unwrap();

    let request_head = git_head_oid(&source);
    state
        .metadata
        .read(|catalog| {
            let request = catalog.requests.get(REQUEST_ID).unwrap();
            assert_eq!(request.head_oid, request_head);
            assert!(request.git_snapshot.is_some());
            assert!(catalog.request_events.is_empty());
            Ok(())
        })
        .unwrap();

    server.abort();
    let _ = fs::remove_dir_all(source);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_ref_push_rejects_history_unrelated_to_recorded_base() {
    let state = test_state_with_request();
    state
        .metadata
        .update(|catalog| {
            let request = catalog.requests.get_mut(REQUEST_ID).unwrap();
            request.author_user_id = test_owner_id();
            request.author_role = RequestActorRole::Owner;
            request.base_audience = RequestBaseAudience::Private;
            Ok(())
        })
        .unwrap();
    let original_head = state
        .metadata
        .request_by_id(REQUEST_ID)
        .unwrap()
        .unwrap()
        .head_oid;
    let state_for_server = state.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state_for_server))
            .await
            .unwrap();
    });

    let source = temp_checkout_dir("request-ref-unrelated-history");
    let public_remote = format!("http://{addr}/git/public/{TEST_REPO_ID}");
    run_git(
        None,
        &["clone", &public_remote, source.to_str().unwrap()],
        "clone public repo for unrelated request",
    )
    .unwrap();
    run_git(
        Some(&source),
        &["checkout", "--orphan", "unrelated-request"],
        "create unrelated request history",
    )
    .unwrap();
    run_git(
        Some(&source),
        &["rm", "-rf", "."],
        "clear unrelated request tree",
    )
    .unwrap();
    fs::write(source.join("unrelated.txt"), "unrelated history\n").unwrap();
    run_git(
        Some(&source),
        &["add", "-A"],
        "add unrelated request changes",
    )
    .unwrap();
    commit_all(&source, "unrelated request change");
    let permissioned_remote = format!("http://{addr}/git/permissioned/{TEST_REPO_ID}");
    configure_bearer_header(
        &source,
        &permissioned_remote,
        &bearer_header_for(TEST_CLERK_USER_ID, TEST_OWNER_EMAIL),
    );

    let output = run_git_output(
        Some(&source),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "push unrelated request ref",
    )
    .unwrap();

    assert!(!output.status.success());
    state
        .metadata
        .read(|catalog| {
            let request = catalog.requests.get(REQUEST_ID).unwrap();
            assert_eq!(request.head_oid, original_head);
            assert!(request.git_snapshot.is_none());
            assert!(catalog.request_events.is_empty());
            Ok(())
        })
        .unwrap();

    server.abort();
    let _ = fs::remove_dir_all(source);
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
            assert_eq!(request.state, RequestState::Working);
            assert_eq!(request.head_oid, request.base_main_oid);
            assert!(request.git_snapshot.is_none());
            assert!(catalog.request_events.is_empty());
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
            let request = catalog.requests.get(REQUEST_ID).unwrap();
            assert_eq!(request.state, RequestState::Working);
            assert_eq!(request.head_oid, request.base_main_oid);
            assert!(catalog.request_events.is_empty());
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
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Public,
    );
    let projection_repo = projection_bare_repo_for_state(&state, &projection).unwrap();
    let base_main_oid = git_stdout_text(
        &projection_repo,
        &["rev-parse", &format!("refs/heads/{DEFAULT_GIT_BRANCH}")],
        "read request base",
    )
    .unwrap()
    .trim()
    .to_string();
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
        .start_request(StartRequestInput {
            id: REQUEST_ID.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            author_user_id: public_user_id(),
            title: "Request branch".to_string(),
            author_role: RequestActorRole::Public,
            base_audience: RequestBaseAudience::Public,
            target_branch: DEFAULT_GIT_BRANCH.to_string(),
            request_ref: REQUEST_REF.to_string(),
            base_main_oid,
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
                    editor_user_ids: Default::default(),
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

fn insert_editor_user(state: &AppState) {
    let editor = UserAccount {
        id: editor_user_id(),
        handle: "editor".to_string(),
        email: EDITOR_EMAIL.to_string(),
        email_verified: true,
    };
    state
        .metadata
        .update(|catalog| {
            catalog.users.insert(editor.id.clone(), editor);
            Ok(())
        })
        .unwrap();
}

fn insert_stranger_user(state: &AppState) {
    let stranger = UserAccount {
        id: stranger_user_id(),
        handle: "stranger".to_string(),
        email: STRANGER_EMAIL.to_string(),
        email_verified: true,
    };
    state
        .metadata
        .update(|catalog| {
            catalog.users.insert(stranger.id.clone(), stranger);
            Ok(())
        })
        .unwrap();
}

fn insert_member_user(state: &AppState) {
    let member = UserAccount {
        id: member_user_id(),
        handle: "member".to_string(),
        email: MEMBER_EMAIL.to_string(),
        email_verified: true,
    };
    state
        .metadata
        .update(|catalog| {
            catalog.users.insert(member.id.clone(), member);
            catalog
                .repositories
                .get_mut(TEST_REPO_ID)
                .unwrap()
                .members
                .push(test_repository_member(
                    TEST_REPO_ID,
                    member_user_id(),
                    RepositoryMemberPermissions::default(),
                ));
            Ok(())
        })
        .unwrap();
}

fn invite_editor_to_request(state: &AppState) {
    state
        .metadata
        .add_request_editor(AddRequestEditorInput {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: test_owner_id(),
            editor_user_id: editor_user_id(),
            now_unix: 3,
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

fn editor_user_id() -> String {
    crate::db::scope_user_id_for_auth_identity("clerk", EDITOR_SUBJECT)
}

fn stranger_user_id() -> String {
    crate::db::scope_user_id_for_auth_identity("clerk", STRANGER_SUBJECT)
}

fn member_user_id() -> String {
    crate::db::scope_user_id_for_auth_identity("clerk", MEMBER_SUBJECT)
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
