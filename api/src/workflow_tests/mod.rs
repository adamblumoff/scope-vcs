use crate::{
    app::router,
    auth::{clerk::*, tokens::*},
    config::*,
    git::{import::*, projection_repo::*, storage::*, upload::*, *},
    http::responses::*,
    push_intents::*,
    repo_access::*,
    runtime_budgets::{BudgetedObjectStore, RuntimeBudgetConfig, RuntimeBudgets},
    state::*,
    use_cases::{
        content_cleanup::*,
        git_receive::{self as git_receive_use_case, ReceivePackAccess},
    },
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
use scope_domain::policy::{Policy, ScopePath, Visibility, VisibilityRule};
use scope_domain::projection::{
    FileChange, LogicalCommit, ProjectionViewKey, SourceGraph, project_graph,
};
use scope_domain::{
    account::UserAccount,
    projection::LogicalCommitOrigin,
    repo_actions::RepoStorageCleanup,
    repo_config::{ConfigVisibility, RepoConfig},
    repository::collaboration::{
        RepositoryInvite, RepositoryInviteState, RepositoryMember, RepositoryMemberPermissions,
    },
    repository::credentials::GitPushToken,
    repository::{RepoLifecycleState, RepoRecord, Repository},
};
use scope_object_store::{
    ContentObjectKind, MemoryObjectStore, put_content_object, put_source_blob, source_blob_bytes,
};
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
mod cloud_runs;
mod device_login;
mod git_binary;
mod git_http;
mod git_http_gzip;
mod git_import_validation;
mod git_projection_identity;
mod git_receive;
mod git_receive_config;
mod git_request_refs;
mod history;
mod landing_file;
mod push_intent_completion;
mod repo_cleanup;
mod repo_events;
mod repo_lifecycle;
mod repo_visibility;
mod request_discussions;
mod requests;
mod run_inspection;
mod run_resources;
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
    sign_claims_with_kid(claims, "test-key")
}

fn sign_claims_with_kid(claims: serde_json::Value, kid: &str) -> String {
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(kid.into());
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
    scope_postgres::db::scope_user_id_for_auth_identity("clerk", TEST_CLERK_USER_ID)
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
        .admin()
        .seed_catalog_for_tests(scope_postgres::db::CatalogFixture {
            users: BTreeMap::from([(owner.id.clone(), owner)]),
            repositories: BTreeMap::from([(repo.record.id.clone(), repo)]),
            ..Default::default()
        })
        .unwrap();
    state
}

async fn replace_test_repo(state: &AppState, repo: Repository) {
    state
        .metadata
        .repositories()
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
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.record.lifecycle_state = RepoLifecycleState::AwaitingFirstPush;
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
    state.clerk.cache_jwks_for_tests(test_jwks());
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
    fs::create_dir_all(repo.join(".scope")).unwrap();
    fs::write(repo.join(".scope/RULES.md"), []).unwrap();
    run_git(
        Some(&repo),
        &["add", ".scope/RULES.md"],
        "stage canonical repo rules",
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
                .map(|head| head.manifest.content_ref.clone()),
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
    let repo = find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    repo.live_tree()
        .get(&ScopePath::parse(path).unwrap())
        .map(|blob| blob_content(state, blob, &repo))
}

async fn persist_test_update(
    state: &AppState,
    update: impl Into<TestReceivePackUpdate>,
) -> Result<scope_domain::repository::git::GitHead, crate::error::ApiError> {
    persist_and_promote_test_update(state, update, &test_owner_id()).await
}

fn test_git_segment_ref(label: &str) -> scope_domain::repository::git::GitSegmentRef {
    use sha2::{Digest, Sha256};

    scope_domain::repository::git::GitSegmentRef {
        segment_id: format!("test-{}", hex::encode(Sha256::digest(label.as_bytes()))),
        sha256: hex::encode(Sha256::digest(format!("pack:{label}").as_bytes())),
        plaintext_bytes: label.len() as u64,
        encoding_version: scope_git_storage::ENCODING_VERSION,
    }
}

async fn ready_test_git_segment(
    state: &AppState,
    label: &str,
) -> scope_domain::repository::git::GitSegmentRef {
    let reservation = state.git_segment_store.reserve(TEST_REPO_ID).unwrap();
    state
        .metadata
        .repositories()
        .begin_git_segment_upload(
            TEST_REPO_ID,
            &reservation.segment_id,
            &reservation.object_key,
            scope_git_storage::ENCODING_VERSION,
            crate::persistence::unix_now().unwrap(),
        )
        .await
        .unwrap();
    let staged = state
        .git_segment_store
        .ingest_reserved_blocking_reader(
            TEST_REPO_ID,
            reservation,
            std::io::Cursor::new(format!("test segment {label}").into_bytes()),
        )
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .mark_git_segment_upload_ready(
            &staged.segment,
            staged.encrypted_bytes,
            crate::persistence::unix_now().unwrap(),
        )
        .await
        .unwrap();
    state.git_segment_store.delete_local(&staged).await.unwrap();
    staged.segment
}

enum TestReceivePackUpdate {
    Prepared(Box<PreparedReceivePackUpdate>),
    Raw(Box<ReceivePackUpdate>),
}

impl From<PreparedReceivePackUpdate> for TestReceivePackUpdate {
    fn from(update: PreparedReceivePackUpdate) -> Self {
        Self::Prepared(Box::new(update))
    }
}

impl From<ReceivePackUpdate> for TestReceivePackUpdate {
    fn from(update: ReceivePackUpdate) -> Self {
        Self::Raw(Box::new(update))
    }
}

async fn persist_and_promote_test_update(
    state: &AppState,
    update: impl Into<TestReceivePackUpdate>,
    actor_id: &str,
) -> Result<scope_domain::repository::git::GitHead, crate::error::ApiError> {
    let prepared = match update.into() {
        TestReceivePackUpdate::Prepared(prepared) => *prepared,
        TestReceivePackUpdate::Raw(update) => {
            let mut update = *update;
            let reservation = state.git_segment_store.reserve(TEST_REPO_ID).unwrap();
            state
                .metadata
                .repositories()
                .begin_git_segment_upload(
                    TEST_REPO_ID,
                    &reservation.segment_id,
                    &reservation.object_key,
                    scope_git_storage::ENCODING_VERSION,
                    crate::persistence::unix_now()?,
                )
                .await?;
            let staged_segment = state
                .git_segment_store
                .ingest_reserved_blocking_reader(
                    TEST_REPO_ID,
                    reservation,
                    std::io::Cursor::new(b"test Git pack segment".to_vec()),
                )
                .await
                .map_err(|error| crate::error::ApiError::internal_message(error.to_string()))?;
            state
                .metadata
                .repositories()
                .mark_git_segment_upload_ready(
                    &staged_segment.segment,
                    staged_segment.encrypted_bytes,
                    crate::persistence::unix_now()?,
                )
                .await?;
            update.git_pack_span.segment = staged_segment.segment.clone();
            let content_refs = update
                .durable_objects
                .iter()
                .map(|object| object.content_ref.clone())
                .collect::<Vec<_>>();
            let fence = state
                .metadata
                .acquire_content_ref_fence(&content_refs)
                .await?;
            let write_lease = state
                .metadata
                .repositories()
                .acquire_git_write_lease(TEST_REPO_ID)
                .await?;
            let upload_heartbeat = crate::git::import::GitSegmentUploadHeartbeat::start(
                state,
                staged_segment.segment.segment_id.clone(),
            );
            PreparedReceivePackUpdate {
                update,
                fence,
                staged_segment,
                write_lease,
                upload_heartbeat,
            }
        }
    };
    let persisted = git_receive_use_case::main_push::persist_main_push(
        state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        prepared,
        actor_id,
    )
    .await?;
    let head = persisted.head;
    state
        .git_segment_store
        .delete_local(&persisted.staged_segment)
        .await
        .map_err(|error| crate::error::ApiError::internal_message(error.to_string()))?;
    persisted.write_lease.release().await;
    Ok(head)
}

async fn receive_pack_access(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
) -> Result<ReceivePackAccess, crate::error::ApiError> {
    let (authorization, push_intent) = crate::git::receive_pack_credentials(state, headers).await?;
    git_receive_use_case::authorize(
        state,
        owner,
        repo_name,
        authorization,
        push_intent.as_deref(),
    )
    .await
}

async fn published_staging_repo(state: &AppState) -> PathBuf {
    ensure_ready_receive_pack_staging_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME, &test_owner_id())
        .await
        .unwrap()
}

fn git_head_oid(repo: &FsPath) -> String {
    git_stdout_text(repo, &["rev-parse", "HEAD"], "read git head")
        .unwrap()
        .trim()
        .to_string()
}

fn test_repo(owner_id: &str) -> Repository {
    Repository {
        record: RepoRecord {
            id: TEST_REPO_ID.to_string(),
            owner_handle: TEST_REPO_OWNER.to_string(),
            name: TEST_REPO_NAME.to_string(),
            owner_user_id: owner_id.to_string(),
            lifecycle_state: RepoLifecycleState::Ready,
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
        visibility_change_sets: Vec::new(),
        live_files: BTreeMap::new(),
        git_head: None,
        git_pack_spans: Vec::new(),
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

fn source_blob(state: &AppState, content: &str) -> scope_domain::content::SourceBlob {
    source_blob_from_bytes(state, content.as_bytes())
}

fn source_blob_from_bytes(state: &AppState, bytes: &[u8]) -> scope_domain::content::SourceBlob {
    put_source_blob(state.object_store.as_ref(), bytes).unwrap()
}

fn blob_content(
    state: &AppState,
    blob: &scope_domain::content::SourceBlob,
    repo: &Repository,
) -> String {
    let git_source = repo.git_head.as_ref().map(|head| {
        (
            repo.record.id.as_str(),
            head,
            repo.git_pack_spans.as_slice(),
        )
    });
    String::from_utf8(crate::git::content::source_content_bytes(state, blob, git_source).unwrap())
        .unwrap()
}

fn repo_with_readme(state: &AppState) -> Repository {
    let mut repo = test_repo(&test_owner_id());
    let path = ScopePath::parse("/README.md").unwrap();
    let content = source_blob(state, "hello");
    let rules_path = ScopePath::parse("/.scope/RULES.md").unwrap();
    let rules_content = source_blob(state, "");
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: "rv1".to_string(),
        },
        author_id: repo.record.owner_user_id.clone(),
        message: "initial".to_string(),
        changes: vec![
            FileChange {
                visibility: Visibility::Public,
                path: path.clone(),
                old_content: None,
                new_content: Some(content.clone()),
            },
            FileChange {
                visibility: Visibility::Public,
                path: rules_path.clone(),
                old_content: None,
                new_content: Some(rules_content.clone()),
            },
        ],
    });
    repo.live_files.insert(path, content);
    repo.live_files.insert(rules_path, rules_content);
    repo
}

fn populate_test_live_files(repo: &mut Repository) {
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
    let mut manifest = put_content_object(
        state.object_store.as_ref(),
        ContentObjectKind::GitManifest,
        b"test staged Git manifest",
    )
    .unwrap();
    let head_oid = "1111111111111111111111111111111111111111";
    manifest.git_oid = head_oid.to_string();
    ReceivePackUpdate {
        branch: format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        head_oid: head_oid.to_string(),
        base_git_manifest_ref: None,
        author_id: test_owner_id(),
        message: "owner push".to_string(),
        git_head: scope_domain::repository::git::GitHead {
            head_oid: "1111111111111111111111111111111111111111".to_string(),
            push_sequence: 1,
            change_version: 1,
            manifest,
        },
        git_pack_span: scope_domain::repository::git::GitPackSpan {
            first_sequence: 1,
            last_sequence: 1,
            geometric_tier: 0,
            base_oid: None,
            head_oid: "1111111111111111111111111111111111111111".to_string(),
            segment: test_git_segment_ref("test staged Git segment"),
        },
        durable_objects: Vec::new(),
        workflow_catalog: scope_domain::runs::catalog::RepositoryWorkflowCatalog::captured(
            TEST_REPO_ID,
            head_oid,
            2,
            Vec::new(),
        )
        .unwrap(),
        landing_file_mutation: scope_domain::landing_file::RepositoryLandingFileMutation::Unchanged,
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
