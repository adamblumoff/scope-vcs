use super::*;
use scope_domain::requests::{
    Request, RequestActorRole, RequestAudience, RequestEventKind, RequestState, StartRequestInput,
    SubmitRequestInput,
};
use scope_postgres::db::AddRequestInviteeCommand;

const PUBLIC_SUBJECT: &str = "user_public";
const PUBLIC_EMAIL: &str = "public@example.com";
const CONTRIBUTOR_SUBJECT: &str = "user_contributor";
const CONTRIBUTOR_EMAIL: &str = "contributor@example.com";
const MEMBER_SUBJECT: &str = "user_member";
const MEMBER_EMAIL: &str = "member@example.com";
const UNRELATED_SUBJECT: &str = "user_unrelated";
const UNRELATED_EMAIL: &str = "unrelated@example.com";
const REQUEST_ID: &str = "req_1";
const REQUEST_NAME: &str = "request-branch";
const REQUEST_REF: &str = "refs/heads/request-branch";
const PRIVATE_REQUEST_ID: &str = "req_private";
const PRIVATE_REQUEST_REF: &str = "refs/heads/private-request";

mod http;
mod merge;
mod policy;
mod privacy;
mod refs;
mod review;

use http::public_get_json;
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

async fn test_state_with_request() -> AppState {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(public_user_id(), "public", PUBLIC_EMAIL))
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .replace_repository_for_tests(repo_with_readme(&state))
        .await
        .unwrap();
    start_public_request(&state).await;
    state
}

async fn test_state_with_mergeable_request() -> (AppState, TempGitRepo) {
    let (state, source, _head) =
        super::push_intent_completion::published_git_fixture("request-merge-state").await;
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(public_user_id(), "public", PUBLIC_EMAIL))
        .await
        .unwrap();
    start_public_request(&state).await;
    (state, source)
}

async fn test_state_with_mergeable_owner_public_request() -> AppState {
    let (state, _source, _head) =
        super::push_intent_completion::published_git_fixture("owner-public-request-merge-state")
            .await;
    start_request_for_author(&state, test_owner_id(), RequestActorRole::Owner).await;
    state
}

async fn start_public_request(state: &AppState) {
    start_request_for_author(state, public_user_id(), RequestActorRole::Public).await;
}

async fn start_request_for_author(
    state: &AppState,
    author_user_id: String,
    author_role: RequestActorRole,
) {
    let repo = find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let projection = project_graph(
        &repo.graph,
        &repo.visibility_change_sets,
        ProjectionViewKey::Public,
    );
    let projection_repo = projection_bare_repo_for_state(
        state,
        &repo.record.id,
        &projection,
        repo.git_head.as_ref(),
        &repo.git_pack_spans,
    )
    .unwrap();
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
        .requests()
        .start_request(StartRequestInput {
            id: REQUEST_ID.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: REQUEST_NAME.to_string(),
            author_user_id,
            title: Some("Request branch".to_string()),
            author_role,
            audience: RequestAudience::Public,
            base_main_oid,
            event_id: "event_request_branch_started".to_string(),
            now_unix: 2,
        })
        .await
        .unwrap();
}

async fn insert_private_request_for_public_user(state: &AppState) {
    state
        .metadata
        .requests()
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
            activity_version: 0,
            submitted_at_unix: None,
            closed_at_unix: None,
            closed_by_user_id: None,
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
        .auth()
        .insert_user_for_tests(test_user(member_user_id(), "member", MEMBER_EMAIL))
        .await
        .unwrap();
    state
        .metadata
        .repositories()
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
        .auth()
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
    assert_eq!(request.state(), RequestState::Draft);
    assert_eq!(request.head_oid, request.base_main_oid);
    assert!(request.git_snapshot.is_none());
    assert_eq!(request_event_count(state).await, 1);
}

async fn stored_request(state: &AppState, id: &str) -> Request {
    state
        .metadata
        .requests()
        .request_for_tests(id)
        .await
        .unwrap()
        .unwrap()
}

async fn request_event_count(state: &AppState) -> usize {
    state
        .metadata
        .requests()
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
    scope_postgres::db::scope_user_id_for_auth_identity("clerk", PUBLIC_SUBJECT)
}

fn contributor_user_id() -> String {
    scope_postgres::db::scope_user_id_for_auth_identity("clerk", CONTRIBUTOR_SUBJECT)
}

fn member_user_id() -> String {
    scope_postgres::db::scope_user_id_for_auth_identity("clerk", MEMBER_SUBJECT)
}

fn checkout_dir(label: &str) -> TempGitRepo {
    TempGitRepo(unique_test_path(label))
}
