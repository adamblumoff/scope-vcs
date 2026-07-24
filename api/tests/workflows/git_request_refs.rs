use super::*;
use crate::domain::requests::{
    Request, RequestActorRole, RequestAudience, RequestState, StartRequestInput,
};
use tokio::sync::broadcast::error::TryRecvError;

const PUBLIC_SUBJECT: &str = "user_public";
const PUBLIC_EMAIL: &str = "public@example.com";
const CONTRIBUTOR_SUBJECT: &str = "user_contributor";
const CONTRIBUTOR_EMAIL: &str = "contributor@example.com";
const MEMBER_SUBJECT: &str = "user_member";
const MEMBER_EMAIL: &str = "member@example.com";
const REQUEST_ID: &str = "req_1";
const REQUEST_NAME: &str = "request-branch";
const REQUEST_REF: &str = "refs/heads/request-branch";
const PRIVATE_REQUEST_ID: &str = "req_private";
const PRIVATE_REQUEST_REF: &str = "refs/heads/private-request";

mod privacy;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permissioned_clone_fetches_named_public_requests_without_joining() {
    let state = test_state_with_request().await;
    insert_public_contributor(&state).await;
    let (origin, _server) = spawn_test_server(&state).await;
    let checkout = checkout_dir("named-request-clone");
    let permissioned_remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}");
    clone_with_bearer(
        &permissioned_remote,
        &checkout,
        &bearer_header_for(CONTRIBUTOR_SUBJECT, CONTRIBUTOR_EMAIL),
        "clone all public request refs",
    );

    let request_head = git_stdout_text(
        &checkout,
        &["rev-parse", "refs/remotes/origin/request-branch"],
        "read fetched request ref",
    )
    .unwrap();
    assert_eq!(
        request_head.trim(),
        stored_request(&state, REQUEST_ID).await.head_oid
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closed_public_request_remains_fetchable_as_read_only_history() {
    let state = test_state_with_request().await;
    insert_public_contributor(&state).await;
    state
        .metadata
        .mutate_request_for_tests(REQUEST_ID, |request| {
            request.state = RequestState::Completed;
            request.first_ready_at_unix = Some(3);
            request.completed_at_unix = Some(3);
            request.completed_by_user_id = Some(test_owner_id());
            request.updated_at_unix = 3;
        })
        .await
        .unwrap();
    let (origin, _server) = spawn_test_server(&state).await;
    let checkout = checkout_dir("closed-named-request-clone");
    let permissioned_remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}");
    clone_with_bearer(
        &permissioned_remote,
        &checkout,
        &bearer_header_for(CONTRIBUTOR_SUBJECT, CONTRIBUTOR_EMAIL),
        "clone closed public request ref",
    );

    assert!(
        git_stdout_text(
            &checkout,
            &["rev-parse", "refs/remotes/origin/request-branch"],
            "read closed request ref",
        )
        .is_ok()
    );
    fs::write(checkout.join("closed.txt"), "closed request edit\n").unwrap();
    run_git(Some(&checkout), &["add", "closed.txt"], "stage closed edit").unwrap();
    commit_all(&checkout, "closed request edit");
    let output = run_git_output(
        Some(&checkout),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "reject closed request push",
    )
    .unwrap();
    assert!(!output.status.success());
}

#[tokio::test]
async fn public_request_receive_pack_requires_current_repo_read() {
    let state = test_state_with_request().await;
    state
        .metadata
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.policy = Policy::new(Visibility::Private);
            repo.graph.commits.clear();
        })
        .await
        .unwrap();
    let headers = authorization_headers(bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL));

    let error = receive_pack_access(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap_err();

    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn working_request_ref_push_replaces_snapshot_without_touching_main() {
    let state = test_state_with_request().await;
    let (source, permissioned_remote, _server, first_request_head) =
        request_checkout(&state, "request-ref-push").await;
    push_change(
        &source,
        &permissioned_remote,
        REQUEST_REF,
        "request.txt",
        "request branch content v2\n",
        "request change v2",
    )
    .unwrap();
    let request_head = git_head_oid(&source);

    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("hello")
    );
    assert_eq!(live_file_content(&state, "/request.txt").await, None);
    let request = stored_request(&state, REQUEST_ID).await;
    assert_eq!(request.head_oid, request_head);
    source_blob_bytes(
        state.object_store.as_ref(),
        request.git_snapshot.as_ref().unwrap(),
    )
    .unwrap();
    assert_eq!(request_event_count(&state).await, 1);
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
    let staging_repo = assert_restored_request_head(&state, &request_head).await;
    let _ = fs::remove_dir_all(staging_repo);
    fs::remove_dir_all(&store_repo).unwrap();
    let staging_repo = assert_restored_request_head(&state, &request_head).await;
    let _ = fs::remove_dir_all(staging_repo);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn working_push_does_not_create_review_activity() {
    let state = test_state_with_request().await;
    let (source, permissioned_remote, _server, _) =
        request_checkout(&state, "request-ref-revision").await;
    let before = stored_request(&state, REQUEST_ID).await;
    let before_event_count = request_event_count(&state).await;
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    push_change(
        &source,
        &permissioned_remote,
        REQUEST_REF,
        "request.txt",
        "request branch content after feedback\n",
        "respond with revision",
    )
    .unwrap();

    let request = stored_request(&state, REQUEST_ID).await;
    assert_eq!(request.state, RequestState::Working);
    assert_eq!(request.head_oid, git_head_oid(&source));
    assert_eq!(request.activity_version, before.activity_version);
    assert_eq!(request_event_count(&state).await, before_event_count);
    assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ready_revision_rejection_rolls_back_without_publishing_refreshes() {
    let state = test_state_with_request().await;
    let (source, permissioned_remote, _server, first_request_head) =
        request_checkout(&state, "request-ref-revision-rollback").await;
    state
        .metadata
        .mutate_request_for_tests(REQUEST_ID, |request| {
            request.state = RequestState::ReadyForReview;
            request.current_stake_credits = 10;
            request.first_ready_at_unix = Some(4);
            request.ready_at_unix = Some(4);
            request.updated_at_unix = 4;
        })
        .await
        .unwrap();
    let before = stored_request(&state, REQUEST_ID).await;
    let before_event_count = request_event_count(&state).await;
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);

    let output = push_change(
        &source,
        &permissioned_remote,
        REQUEST_REF,
        "request.txt",
        "request branch content that must roll back\n",
        "revision that overflows persistence",
    )
    .unwrap_err();

    assert!(!output.status.success());
    let after = stored_request(&state, REQUEST_ID).await;
    assert_eq!(after, before);
    assert_eq!(request_event_count(&state).await, before_event_count);
    let store_repo =
        crate::git::storage::request_ref_store_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME);
    let stored_head = git_stdout_text(
        &store_repo,
        &["rev-parse", REQUEST_REF],
        "read rolled back request ref",
    )
    .unwrap();
    assert_eq!(stored_head.trim(), first_request_head);
    assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));
}

async fn assert_restored_request_head(state: &AppState, expected: &str) -> PathBuf {
    let staging = crate::git::request_refs::ensure_request_receive_pack_staging_repo(
        state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &public_user_id(),
    )
    .await
    .unwrap();
    let head = git_stdout_text(&staging, &["rev-parse", REQUEST_REF], "read request ref")
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(head, expected);
    staging
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn any_public_contributor_and_maintainer_can_push_request_refs() {
    for (label, subject, email, path, prepare) in [
        (
            "request-ref-contributor-push",
            CONTRIBUTOR_SUBJECT,
            CONTRIBUTOR_EMAIL,
            "contributor.txt",
            true,
        ),
        (
            "request-ref-maintainer-push",
            MEMBER_SUBJECT,
            MEMBER_EMAIL,
            "maintainer.txt",
            false,
        ),
    ] {
        let state = test_state_with_request().await;
        if prepare {
            insert_public_contributor(&state).await;
        } else {
            insert_member_user(&state).await;
        }
        let (source, remote, _server) = request_push_checkout(&state, label, subject, email).await;
        push_change(
            &source,
            &remote,
            REQUEST_REF,
            path,
            "request branch content\n",
            "request change",
        )
        .unwrap();

        let request = stored_request(&state, REQUEST_ID).await;
        assert_eq!(request.head_oid, git_head_oid(&source));
        assert!(request.git_snapshot.is_some());
        assert_eq!(request_event_count(&state).await, 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_ref_push_rejects_history_unrelated_to_recorded_base() {
    let state = test_state_with_request().await;
    state
        .metadata
        .mutate_request_for_tests(REQUEST_ID, |request| {
            request.author_user_id = test_owner_id();
            request.author_role = RequestActorRole::Owner;
            request.audience = RequestAudience::Private;
        })
        .await
        .unwrap();
    let (source, permissioned_remote, _server) = request_push_checkout(
        &state,
        "request-ref-unrelated-history",
        TEST_CLERK_USER_ID,
        TEST_OWNER_EMAIL,
    )
    .await;
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
    let output = run_git_output(
        Some(&source),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "push unrelated request ref",
    )
    .unwrap();

    assert!(!output.status.success());
    assert_request_branch_unchanged(&state).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_ref_push_rejects_unsupported_tree_entries() {
    let state = test_state_with_request().await;
    let (source, permissioned_remote, _server) = request_push_checkout(
        &state,
        "request-ref-invalid-tree",
        PUBLIC_SUBJECT,
        PUBLIC_EMAIL,
    )
    .await;
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
    let output = run_git_output(
        Some(&source),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "push invalid request ref",
    )
    .unwrap();

    assert!(!output.status.success());
    assert_request_branch_unchanged(&state).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_request_author_cannot_push_main() {
    let state = test_state_with_request().await;
    let (source, permissioned_remote, _server) = request_push_checkout(
        &state,
        "request-main-rejected",
        PUBLIC_SUBJECT,
        PUBLIC_EMAIL,
    )
    .await;
    let output = push_change(
        &source,
        &permissioned_remote,
        "main",
        "README.md",
        "public main write\n",
        "try main",
    )
    .unwrap_err();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Scope contributors cannot update main")
    );
    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("hello")
    );
    assert_request_branch_unchanged(&state).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_actor_cannot_push_private_request_after_membership_loss() {
    let state = test_state_with_request().await;
    insert_private_request_for_public_user(&state).await;
    let (source, permissioned_remote, _server) = request_push_checkout(
        &state,
        "private-request-rejected",
        PUBLIC_SUBJECT,
        PUBLIC_EMAIL,
    )
    .await;
    push_change(
        &source,
        &permissioned_remote,
        PRIVATE_REQUEST_REF,
        "private-request.txt",
        "private request write\n",
        "try private request",
    )
    .unwrap_err();

    assert_eq!(
        stored_request(&state, PRIVATE_REQUEST_ID).await.head_oid,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
}

async fn test_state_with_request() -> AppState {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    state
        .metadata
        .insert_user_for_tests(test_user(public_user_id(), "public", PUBLIC_EMAIL))
        .await
        .unwrap();
    state
        .metadata
        .replace_repository_for_tests(repo_with_readme(&state))
        .await
        .unwrap();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
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
        .start_request(StartRequestInput {
            id: REQUEST_ID.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: REQUEST_NAME.to_string(),
            author_user_id: public_user_id(),
            title: Some("Request branch".to_string()),
            author_role: RequestActorRole::Public,
            audience: RequestAudience::Public,
            base_main_oid,
            event_id: "event_request_branch_started".to_string(),
            now_unix: 2,
        })
        .await
        .unwrap();
    state
}

async fn insert_private_request_for_public_user(state: &AppState) {
    state
        .metadata
        .insert_request_for_tests(Request {
            id: PRIVATE_REQUEST_ID.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: "private-request".to_string(),
            author_user_id: public_user_id(),
            author_role: RequestActorRole::Member,
            audience: RequestAudience::Private,
            base_main_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            head_oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            git_snapshot: None,
            title: "Former member request".to_string(),
            description_markdown: String::new(),
            state: RequestState::Working,
            activity_version: 0,
            current_stake_credits: 0,
            first_ready_at_unix: None,
            ready_at_unix: None,
            held_at_unix: None,
            held_by_user_id: None,
            assessment_outcome: None,
            assessment_body_markdown: None,
            assessed_at_unix: None,
            assessed_by_user_id: None,
            completed_at_unix: None,
            completed_by_user_id: None,
            merged_at_unix: None,
            merged_by_user_id: None,
            merged_head_oid: None,
            merged_main_oid: None,
            created_at_unix: 2,
            updated_at_unix: 2,
        })
        .await
        .unwrap();
}

async fn insert_member_user(state: &AppState) {
    state
        .metadata
        .insert_user_for_tests(test_user(member_user_id(), "member", MEMBER_EMAIL))
        .await
        .unwrap();
    state
        .metadata
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.members.push(test_repository_member(
                TEST_REPO_ID,
                member_user_id(),
                RepositoryMemberPermissions::default(),
            ));
        })
        .await
        .unwrap();
}

async fn insert_public_contributor(state: &AppState) {
    state
        .metadata
        .insert_user_for_tests(test_user(
            contributor_user_id(),
            "contributor",
            CONTRIBUTOR_EMAIL,
        ))
        .await
        .unwrap();
}

async fn assert_request_branch_unchanged(state: &AppState) {
    let request = stored_request(state, REQUEST_ID).await;
    assert_eq!(request.state, RequestState::Working);
    assert_eq!(request.head_oid, request.base_main_oid);
    assert!(request.git_snapshot.is_none());
    assert_eq!(request_event_count(state).await, 1);
}

async fn stored_request(state: &AppState, id: &str) -> Request {
    state.metadata.request_for_tests(id).await.unwrap().unwrap()
}

async fn request_event_count(state: &AppState) -> usize {
    state
        .metadata
        .request_events_for_tests()
        .await
        .unwrap()
        .len()
}

async fn request_checkout(
    state: &AppState,
    label: &str,
) -> (TempGitRepo, String, TestServer, String) {
    let (source, permissioned_remote, server) =
        request_push_checkout(state, label, PUBLIC_SUBJECT, PUBLIC_EMAIL).await;
    push_change(
        &source,
        &permissioned_remote,
        REQUEST_REF,
        "request.txt",
        "request branch content\n",
        "request change",
    )
    .unwrap();
    let first_request_head = git_head_oid(&source);
    (source, permissioned_remote, server, first_request_head)
}

async fn request_push_checkout(
    state: &AppState,
    label: &str,
    subject: &str,
    email: &str,
) -> (TempGitRepo, String, TestServer) {
    let (origin, server) = spawn_test_server(state).await;
    let source = checkout_dir(label);
    let public_remote = format!("{origin}/git/public/{TEST_REPO_ID}");
    run_git(
        None,
        &["clone", &public_remote, source.to_str().unwrap()],
        "clone public repo for request ref",
    )
    .unwrap();
    let permissioned_remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}");
    configure_bearer_header(
        &source,
        &permissioned_remote,
        &bearer_header_for(subject, email),
    );
    (source, permissioned_remote, server)
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

fn push_change(
    repo: &FsPath,
    remote: &str,
    target_ref: &str,
    path: &str,
    content: &str,
    message: &str,
) -> Result<(), std::process::Output> {
    fs::write(repo.join(path), content).unwrap();
    run_git(Some(repo), &["add", "-A"], "stage request change").unwrap();
    commit_all(repo, message);
    let output = run_git_output(
        Some(repo),
        &["push", remote, &format!("HEAD:{target_ref}")],
        "push request change",
    )
    .unwrap();
    if output.status.success() {
        Ok(())
    } else {
        Err(output)
    }
}

fn public_user_id() -> String {
    crate::db::scope_user_id_for_auth_identity("clerk", PUBLIC_SUBJECT)
}

fn contributor_user_id() -> String {
    crate::db::scope_user_id_for_auth_identity("clerk", CONTRIBUTOR_SUBJECT)
}

fn member_user_id() -> String {
    crate::db::scope_user_id_for_auth_identity("clerk", MEMBER_SUBJECT)
}

fn checkout_dir(label: &str) -> TempGitRepo {
    TempGitRepo(unique_test_path(label))
}
