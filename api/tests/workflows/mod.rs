use crate::domain::policy::{Policy, ScopePath, Visibility, VisibilityRule};
use crate::domain::projection::{
    AuthorVisibility, FileChange, LogicalCommit, ProjectionViewKey, SourceGraph, project_graph,
};
use crate::domain::repo_config::{ConfigVisibility, RepoConfig};
use crate::domain::store::{
    AppCatalog, GitPushToken, RepoPublicationState, RepoRecord, RepoStorageCleanup,
    RepositoryInvite, RepositoryInviteState, RepositoryMember, RepositoryMemberPermissions,
    StoredRepository, UserAccount,
};
use crate::{
    app::router,
    auth::{clerk::*, tokens::*},
    config::*,
    git::{import::*, storage::*, upload::*, *},
    http::responses::*,
    object_store::{MemoryObjectStore, put_source_blob, source_blob_bytes},
    runtime_budgets::{BudgetedObjectStore, RuntimeBudgetConfig, RuntimeBudgets},
    state::*,
};
use axum::{
    body::{Body, to_bytes},
    http::{
        HeaderMap, Request, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE},
    },
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode, jwk::JwkSet};
use std::{
    collections::BTreeMap,
    fs,
    ops::Deref,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tower::ServiceExt;

mod admin;
mod auth;
mod cli_auth;
mod clone_access;
mod commit_history;
mod device_login;
mod git_binary;
mod git_http;
mod git_http_gzip;
mod git_import_validation;
mod git_projection_identity;
mod git_receive;
mod git_receive_config;
mod git_request_refs;
mod push_intent_completion;
mod repo_cleanup;
mod repo_events;
mod repo_lifecycle;
mod repo_visibility;
mod request_discussions;
mod requests;
mod runtime_budgets;

const TEST_CLERK_ISSUER: &str = "https://clerk.test";
const TEST_CLERK_AUDIENCE: &str = "scope-api";
const TEST_CLERK_USER_ID: &str = "user_owner";
const TEST_OWNER_EMAIL: &str = "owner@example.com";
const TEST_REPO_OWNER: &str = "owner";
const TEST_REPO_NAME: &str = "repo";
const TEST_REPO_ID: &str = "owner/repo";

const TEST_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgj30p9gYDpHRqbshS
LyBNueRnRb9WS031zFD7yuhqn/ChRANCAAR6wR8PANHsn10BAVi085aM8LBPL3Cj
kGxvBjzgF9RjXJoldYnFk7mJ5gLANHjaaad3qTQJ8DldKJoSqkEkm5gg
-----END PRIVATE KEY-----"#;

const TEST_JWKS: &str = r#"{
  "keys": [{
    "kty": "EC",
    "x": "esEfDwDR7J9dAQFYtPOWjPCwTy9wo5BsbwY84BfUY1w",
    "y": "miV1icWTuYnmAsA0eNppp3epNAnwOV0omhKqQSSbmCA",
    "crv": "P-256",
    "kid": "test-key",
    "use": "sig",
    "alg": "ES256"
  }]
}"#;

fn test_jwks() -> JwkSet {
    serde_json::from_str(TEST_JWKS).unwrap()
}

fn sign_claims(claims: serde_json::Value) -> String {
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some("test-key".into());
    encode(
        &header,
        &claims,
        &EncodingKey::from_ec_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap(),
    )
    .unwrap()
}

fn token(user_id: &str, email_verified: bool) -> String {
    token_for_claims(
        user_id,
        Some(TEST_OWNER_EMAIL.to_string()),
        email_verified,
        Some(LOCAL_APP_ORIGIN),
        None,
    )
}

fn token_with_audience(user_id: &str, aud: serde_json::Value) -> String {
    token_for_claims(
        user_id,
        Some(TEST_OWNER_EMAIL.to_string()),
        true,
        Some(LOCAL_APP_ORIGIN),
        Some(aud),
    )
}

fn token_for_claims(
    user_id: &str,
    email: Option<String>,
    email_verified: bool,
    azp: Option<&str>,
    aud: Option<serde_json::Value>,
) -> String {
    let mut claims = serde_json::json!({
        "iss": TEST_CLERK_ISSUER,
        "exp": unix_now() + 300,
        "sub": user_id,
        "email": email,
        "email_verified": email_verified,
    });
    if let Some(azp) = azp {
        claims["azp"] = serde_json::json!(azp);
    }
    if let Some(aud) = aud {
        claims["aud"] = aud;
    }

    sign_claims(claims)
}

fn test_clerk_policy() -> ClerkTokenPolicy {
    ClerkTokenPolicy {
        authorized_parties: vec![LOCAL_APP_ORIGIN.to_string()],
        audiences: vec![TEST_CLERK_AUDIENCE.to_string()],
    }
}

fn token_without_required_claims() -> String {
    sign_claims(serde_json::json!({
        "exp": unix_now() + 300,
        "email": TEST_OWNER_EMAIL,
        "email_verified": true,
    }))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn test_owner_id() -> String {
    crate::db::scope_user_id_for_auth_identity("clerk", TEST_CLERK_USER_ID)
}

fn test_user(id: impl Into<String>, handle: &str, email: &str) -> UserAccount {
    UserAccount {
        id: id.into(),
        handle: handle.to_string(),
        email: email.to_string(),
        email_verified: true,
    }
}

fn test_state_with_repo() -> AppState {
    let owner_id = test_owner_id();
    let owner = test_user(&owner_id, TEST_REPO_OWNER, TEST_OWNER_EMAIL);
    let repo = test_repo(&owner_id);
    let state = AppState::test_state();
    state
        .metadata
        .seed_catalog_for_tests(AppCatalog {
            users: BTreeMap::from([(owner.id.clone(), owner)]),
            repositories: BTreeMap::from([(repo.record.id.clone(), repo)]),
            ..Default::default()
        })
        .unwrap();
    state
}

async fn replace_test_repo(state: &AppState, repo: StoredRepository) {
    state
        .metadata
        .replace_repository_for_tests(repo)
        .await
        .unwrap();
}

async fn test_state_with_readme() -> AppState {
    let state = test_state_with_repo();
    replace_test_repo(&state, repo_with_readme(&state)).await;
    state
}

async fn test_state_with_git_push_token(secret: &str) -> AppState {
    let state = test_state_with_repo();
    let mut repo = repo_with_readme(&state);
    repo.git_push_token = Some(GitPushToken {
        token_hash: git_push_token_hash(secret),
        owner_user_id: repo.record.owner_user_id.clone(),
        created_at_unix: unix_now(),
    });
    replace_test_repo(&state, repo).await;
    state
}

async fn test_state_with_first_push_token() -> (AppState, String) {
    let state = test_state_with_repo();
    let (secret, token) = generate_first_push_token(&test_owner_id()).unwrap();
    state
        .metadata
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.record.publication_state = RepoPublicationState::Unpublished;
            repo.first_push_token = Some(token);
        })
        .await
        .unwrap();
    (state, secret)
}

fn test_state_with_jwks() -> AppState {
    let state = AppState::test_state();
    cache_test_jwks(&state);
    state
}

fn cache_test_jwks(state: &AppState) {
    *state
        .clerk
        .jwks_cache
        .lock()
        .expect("test JWKS lock must not be poisoned") = Some(test_jwks());
}

fn bearer_header() -> String {
    format!("Bearer {}", api_token(TEST_CLERK_USER_ID, TEST_OWNER_EMAIL))
}

fn bearer_header_for(user_id: &str, email: &str) -> String {
    format!("Bearer {}", api_token(user_id, email))
}

fn api_token(user_id: &str, email: &str) -> String {
    token_for_claims(
        user_id,
        Some(email.to_string()),
        true,
        Some(LOCAL_APP_ORIGIN),
        Some(serde_json::json!(TEST_CLERK_AUDIENCE)),
    )
}

async fn response_json(response: Response) -> serde_json::Value {
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn assert_text_content(value: &serde_json::Value, expected: &str) {
    assert_eq!(value["kind"], "text");
    assert_eq!(value["text"], expected);
}

struct TempGitRepo(PathBuf);

impl Deref for TempGitRepo {
    type Target = FsPath;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<FsPath> for TempGitRepo {
    fn as_ref(&self) -> &FsPath {
        &self.0
    }
}

impl Drop for TempGitRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_git_repo(label: &str) -> TempGitRepo {
    let repo = unique_test_path(label);
    let _ = fs::remove_dir_all(&repo);
    fs::create_dir_all(&repo).unwrap();
    run_git(
        None,
        &["init", "-b", "main", repo.to_str().unwrap()],
        "init test repo",
    )
    .unwrap();
    TempGitRepo(repo)
}

fn unique_test_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "scope-vcs-{label}-{}-{}",
        std::process::id(),
        unix_now()
    ))
}

fn clone_test_repo(source: &FsPath, label: &str, bare: bool) -> TempGitRepo {
    let repo = unique_test_path(label);
    let _ = fs::remove_dir_all(&repo);
    let mut args = vec!["clone"];
    if bare {
        args.push("--bare");
    }
    args.extend([source.to_str().unwrap(), repo.to_str().unwrap()]);
    run_git(None, &args, "clone test repo").unwrap();
    TempGitRepo(repo)
}

fn commit_all(repo: &FsPath, message: &str) {
    run_git(
        Some(repo),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "commit",
            "-m",
            message,
        ],
        "commit test repo",
    )
    .unwrap();
}

fn clone_with_bearer(remote: &str, destination: &FsPath, bearer_header_value: &str, action: &str) {
    let header = format!("http.{remote}.extraHeader=Authorization: {bearer_header_value}");
    run_git(
        None,
        &[
            "-c",
            &header,
            "clone",
            remote,
            destination.to_str().unwrap(),
        ],
        action,
    )
    .unwrap();
}

const TEST_PUSH_HEAD_OID: &str = "1111111111111111111111111111111111111111";

async fn insert_push_intent_header(
    state: &AppState,
    headers: &mut HeaderMap,
    user_id: &str,
    head_oid: &str,
) {
    let token = create_test_push_intent(state, user_id, head_oid).await;
    headers.insert("x-scope-push-intent", token.parse().unwrap());
}

async fn configure_push_intent_header(
    state: &AppState,
    repo: &FsPath,
    remote: &str,
    user_id: &str,
) {
    let head_oid = git_head_oid(repo);
    let token = create_test_push_intent(state, user_id, &head_oid).await;
    let key = format!("http.{remote}.extraHeader");
    run_git(
        Some(repo),
        &[
            "config",
            "--add",
            key.as_str(),
            &format!("X-Scope-Push-Intent: {token}"),
        ],
        "configure push intent header",
    )
    .unwrap();
}

async fn create_test_push_intent(state: &AppState, user_id: &str, head_oid: &str) -> String {
    let repo = find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let config = repo.repo_config.clone();
    state
        .create_push_intent(
            TEST_REPO_ID,
            user_id,
            head_oid,
            config.clone(),
            repo_config_fingerprint(&config).unwrap(),
            repo.git_head
                .as_ref()
                .map(|head| head.manifest.object_key.clone()),
        )
        .unwrap()
        .token
}

fn authorization_headers(value: impl AsRef<str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, value.as_ref().parse().unwrap());
    headers
}

fn git_push_token_headers(secret: &str) -> HeaderMap {
    authorization_headers(format!(
        "Basic {}",
        BASE64.encode(format!("scope:{secret}"))
    ))
}

struct TestServer(tokio::task::JoinHandle<()>);

impl Drop for TestServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn spawn_test_server(state: &AppState) -> (String, TestServer) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let state = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    (origin, TestServer(server))
}

async fn live_file_content(state: &AppState, path: &str) -> Option<String> {
    find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .live_tree()
        .get(&ScopePath::parse(path).unwrap())
        .map(|blob| blob_content(state, blob))
}

async fn persist_test_update(
    state: &AppState,
    update: ReceivePackUpdate,
) -> Result<PersistedReceivePackUpdate, crate::error::ApiError> {
    persist_and_promote_test_update(state, update, &test_owner_id()).await
}

async fn persist_and_promote_test_update(
    state: &AppState,
    update: ReceivePackUpdate,
    actor_id: &str,
) -> Result<PersistedReceivePackUpdate, crate::error::ApiError> {
    persist_receive_pack_update_and_promote(
        state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        actor_id,
    )
    .await
}

async fn published_staging_repo(state: &AppState) -> PathBuf {
    ensure_published_receive_pack_staging_repo(
        state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &test_owner_id(),
    )
    .await
    .unwrap()
}

fn git_head_oid(repo: &FsPath) -> String {
    git_stdout_text(repo, &["rev-parse", "HEAD"], "read git head")
        .unwrap()
        .trim()
        .to_string()
}

fn test_repo(owner_id: &str) -> StoredRepository {
    StoredRepository {
        record: RepoRecord {
            id: TEST_REPO_ID.to_string(),
            owner_handle: TEST_REPO_OWNER.to_string(),
            name: TEST_REPO_NAME.to_string(),
            owner_user_id: owner_id.to_string(),
            publication_state: RepoPublicationState::Published,
            default_visibility: Visibility::Public,
            change_version: 1,
        },
        repo_config: RepoConfig::with_default_visibility(ConfigVisibility::Public),
        first_push_token: None,
        git_push_token: None,
        policy: Policy::new(Visibility::Public),
        graph: SourceGraph {
            repo_id: TEST_REPO_ID.to_string(),
            commits: Vec::new(),
        },
        visibility_events: Vec::new(),
        live_files: BTreeMap::new(),
        git_head: None,
        git_segments: Vec::new(),
        members: Vec::new(),
        invitations: Vec::new(),
    }
}

fn test_repository_member(
    repo_id: impl Into<String>,
    user_id: impl Into<String>,
    permissions: RepositoryMemberPermissions,
) -> RepositoryMember {
    RepositoryMember {
        repo_id: repo_id.into(),
        user_id: user_id.into(),
        permissions,
        created_at_unix: 10,
        updated_at_unix: 10,
    }
}

fn member_permissions(
    can_push: bool,
    can_change_file_visibility: bool,
    can_apply_changes: bool,
) -> RepositoryMemberPermissions {
    RepositoryMemberPermissions {
        can_push,
        can_change_file_visibility,
        can_apply_changes,
    }
}

async fn apply_first_push_from_staging_repo(
    state: &AppState,
    staging_repo: &FsPath,
    config: RepoConfig,
) {
    let update = reviewed_update_from_staging_repo(
        state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        staging_repo,
        &test_owner_id(),
        config,
    )
    .await
    .unwrap();
    persist_test_update(state, update).await.unwrap();
}

fn source_blob(state: &AppState, content: &str) -> crate::domain::store::SourceBlob {
    source_blob_from_bytes(state, content.as_bytes())
}

fn source_blob_from_bytes(state: &AppState, bytes: &[u8]) -> crate::domain::store::SourceBlob {
    put_source_blob(state.object_store.as_ref(), TEST_REPO_ID, bytes).unwrap()
}

fn blob_content(state: &AppState, blob: &crate::domain::store::SourceBlob) -> String {
    String::from_utf8(crate::git::content::source_content_bytes(state, blob).unwrap()).unwrap()
}

fn repo_with_readme(state: &AppState) -> StoredRepository {
    let mut repo = test_repo(&test_owner_id());
    let path = ScopePath::parse("/README.md").unwrap();
    let content = source_blob(state, "hello");
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        parent_ids: Vec::new(),
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: "initial".to_string(),
        changes: vec![FileChange {
            visibility: Visibility::Public,
            path: path.clone(),
            old_content: None,
            new_content: Some(content.clone()),
        }],
    });
    repo.live_files.insert(path, content);
    repo
}

fn populate_test_live_files(repo: &mut StoredRepository) {
    repo.live_files.clear();
    for change in repo.graph.commits.iter().flat_map(|commit| &commit.changes) {
        match &change.new_content {
            Some(content) => {
                repo.live_files.insert(change.path.clone(), content.clone());
            }
            None => {
                repo.live_files.remove(&change.path);
            }
        }
    }
}

fn receive_pack_update(state: &AppState, changes: Vec<(&str, Option<&str>)>) -> ReceivePackUpdate {
    let config = repo_config(Visibility::Public);
    let manifest = source_blob(state, "test staged Git manifest");
    let segment_object = source_blob(state, "test staged Git segment");
    ReceivePackUpdate {
        branch: format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        head_oid: "1111111111111111111111111111111111111111".to_string(),
        base_git_manifest_key: None,
        author_id: test_owner_id(),
        message: "owner push".to_string(),
        git_head: crate::domain::store::GitHead {
            head_oid: "1111111111111111111111111111111111111111".to_string(),
            segment_sequence: 1,
            change_version: 1,
            manifest,
        },
        git_segment: crate::domain::store::GitSegment {
            sequence: 1,
            base_oid: None,
            head_oid: "1111111111111111111111111111111111111111".to_string(),
            object: segment_object,
            manifest: source_blob(state, "test staged Git segment manifest"),
        },
        durable_objects: Vec::new(),
        previous_config: None,
        base_config_hash: repo_config_fingerprint(&config).unwrap(),
        config,
        changes: changes
            .into_iter()
            .map(|(path, content)| ReceivePackFileChange {
                path: repo_scope_path(path).unwrap(),
                content: content.map(|content| source_blob(state, content)),
            })
            .collect(),
    }
}

fn repo_config(default_visibility: Visibility) -> RepoConfig {
    RepoConfig::with_default_visibility(default_visibility.into())
}
