use crate::domain::policy::{
    Policy, Principal, PrincipalKind, ScopePath, Visibility, VisibilityRule,
};
use crate::domain::projection::{
    AuthorVisibility, FileChange, LogicalCommit, ProjectionViewKey, SourceGraph, VisibilityEvent,
    project_graph,
};
use crate::domain::repo_actions::preview_publish_import;
use crate::domain::repo_config::RepoConfig;
use crate::domain::staged_updates::apply_staged_update_to_repo;
use crate::domain::store::{
    AppCatalog, DEFAULT_GIT_FILE_MODE, EXECUTABLE_GIT_FILE_MODE, GitPushToken, PendingImport,
    PendingImportFile, RepoPublicationState, RepoRecord, RepoSettings, RepoStorageCleanup,
    RepositoryInvite, RepositoryInviteState, RepositoryMember, RepositoryMemberPermissions,
    StagedFileChange, StagedFileChangeKind, StagedRepoUpdate, StoredRepository, UserAccount,
};
use crate::{
    app::router,
    auth::{clerk::*, scope::*, tokens::*},
    config::*,
    git::{import::*, storage::*, upload::*, *},
    http::responses::*,
    object_store::{MemoryObjectStore, put_source_blob, source_blob_bytes},
    persistence::*,
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
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tower::ServiceExt;

mod admin;
mod auth;
mod cli_auth;
mod clone_credentials;
mod commit_history;
mod cors;
mod device_login;
mod git_binary;
mod git_http;
mod git_http_gzip;
mod git_import_validation;
mod git_projection_identity;
mod git_receive;
mod git_receive_config;
mod obsolete_routes;
mod readiness;
mod repo_cleanup;
mod repo_events;
mod repo_lifecycle;
mod repo_visibility;
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

fn token(user_id: &str, email_verified: bool) -> String {
    token_for(user_id, Some(TEST_OWNER_EMAIL.to_string()), email_verified)
}

fn token_for(user_id: &str, email: Option<String>, email_verified: bool) -> String {
    token_for_claims(user_id, email, email_verified, Some(LOCAL_APP_ORIGIN), None)
}

fn token_with_authorized_party(user_id: &str, azp: Option<&str>) -> String {
    token_for_claims(user_id, Some(TEST_OWNER_EMAIL.to_string()), true, azp, None)
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
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some("test-key".to_string());
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

    encode(
        &header,
        &claims,
        &EncodingKey::from_ec_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap(),
    )
    .unwrap()
}

fn test_clerk_policy() -> ClerkTokenPolicy {
    ClerkTokenPolicy {
        authorized_parties: vec![LOCAL_APP_ORIGIN.to_string()],
        audiences: vec![TEST_CLERK_AUDIENCE.to_string()],
    }
}

fn token_without_required_claims() -> String {
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some("test-key".to_string());
    let claims = serde_json::json!({
        "exp": unix_now() + 300,
        "email": TEST_OWNER_EMAIL,
        "email_verified": true,
    });

    encode(
        &header,
        &claims,
        &EncodingKey::from_ec_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap(),
    )
    .unwrap()
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

fn test_state_with_repo() -> AppState {
    let owner_id = test_owner_id();
    let owner = UserAccount {
        id: owner_id.clone(),
        handle: TEST_REPO_OWNER.to_string(),
        email: TEST_OWNER_EMAIL.to_string(),
        email_verified: true,
    };
    let repo = test_repo(&owner_id);
    let runtime_budgets = Arc::new(RuntimeBudgets::from_config(Default::default()));

    AppState {
        metadata: crate::db::MetadataStore::memory(AppCatalog {
            users: BTreeMap::from([(owner.id.clone(), owner)]),
            repositories: BTreeMap::from([(repo.record.id.clone(), repo)]),
            pending_repo_storage_deletions: Vec::new(),
            pending_source_blob_deletions: Vec::new(),
        }),
        data_dir: Arc::new(test_data_dir()),
        clerk: ClerkVerifier::new_with_policy(
            Some(TEST_CLERK_ISSUER.to_string()),
            Some("http://127.0.0.1/.well-known/jwks.json".to_string()),
            test_clerk_policy(),
        ),
        object_store: Arc::new(BudgetedObjectStore::new(
            Arc::new(MemoryObjectStore::new()),
            runtime_budgets.clone(),
        )),
        runtime_budgets,
        operator_token: None,
        repo_events: crate::repo_events::RepoChangeBus::default(),
        push_intent_signing_key: Arc::from(b"scope-test-push-intent-signing-key".as_slice()),
    }
}

fn test_state_with_jwks() -> AppState {
    let state = AppState::test_state();
    cache_test_jwks(&state);
    state
}

fn test_state_with_metadata(metadata: crate::db::MetadataStore) -> AppState {
    let runtime_budgets = Arc::new(RuntimeBudgets::from_config(Default::default()));
    let state = AppState {
        metadata,
        data_dir: Arc::new(test_data_dir()),
        clerk: ClerkVerifier::new_with_policy(
            Some(TEST_CLERK_ISSUER.to_string()),
            Some("http://127.0.0.1/.well-known/jwks.json".to_string()),
            test_clerk_policy(),
        ),
        object_store: Arc::new(BudgetedObjectStore::new(
            Arc::new(MemoryObjectStore::new()),
            runtime_budgets.clone(),
        )),
        runtime_budgets,
        operator_token: None,
        repo_events: crate::repo_events::RepoChangeBus::default(),
        push_intent_signing_key: Arc::from(b"scope-test-push-intent-signing-key".as_slice()),
    };
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

fn temp_git_repo(label: &str) -> PathBuf {
    let repo = std::env::temp_dir().join(format!(
        "scope-vcs-{label}-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&repo);
    fs::create_dir_all(&repo).unwrap();
    run_git(
        None,
        &["init", "-b", "main", repo.to_str().unwrap()],
        "init test repo",
    )
    .unwrap();
    repo
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

fn write_scope_repo_config(repo: &FsPath, default_visibility: Visibility) {
    let default = match default_visibility {
        Visibility::Private => "private",
        Visibility::Public => "public",
    };
    fs::create_dir_all(repo.join(".scope")).unwrap();
    let config = serde_json::json!({
        "kind": "scope.repo-config",
        "version": 1,
        "visibility": {
            "default": default,
            "rules": [],
        },
        "history": {
            "rewrites": [],
        },
    });
    fs::write(
        repo.join(".scope/repo.json"),
        format!("{}\n", serde_json::to_string_pretty(&config).unwrap()),
    )
    .unwrap();
}

const TEST_PUSH_HEAD_OID: &str = "1111111111111111111111111111111111111111";

fn insert_push_intent_header(
    state: &AppState,
    headers: &mut HeaderMap,
    user_id: &str,
    head_oid: &str,
) {
    let token = create_test_push_intent(state, user_id, head_oid);
    headers.insert("x-scope-push-intent", token.parse().unwrap());
}

fn configure_push_intent_header(state: &AppState, repo: &FsPath, remote: &str, user_id: &str) {
    let head_oid = git_head_oid(repo);
    let token = create_test_push_intent(state, user_id, &head_oid);
    let key = format!("http.{remote}.extraHeader");
    run_git(
        Some(repo),
        &[
            "config",
            key.as_str(),
            &format!("X-Scope-Push-Intent: {token}"),
        ],
        "configure push intent header",
    )
    .unwrap();
}

fn create_test_push_intent(state: &AppState, user_id: &str, head_oid: &str) -> String {
    let repo = find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    state
        .create_push_intent(
            TEST_REPO_ID,
            user_id,
            head_oid,
            repo.git_snapshot
                .as_ref()
                .map(|snapshot| snapshot.object_key.clone()),
        )
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
        settings: RepoSettings::default(),
        first_push_token: None,
        git_push_token: None,
        git_clone_tokens: Vec::new(),
        pending_import: None,
        policy: Policy::new(Visibility::Public),
        graph: SourceGraph {
            repo_id: TEST_REPO_ID.to_string(),
            commits: Vec::new(),
        },
        visibility_events: Vec::new(),
        git_snapshot: None,
        staged_update: None,
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

fn pending_import_fixture(files: Vec<(&str, &str)>) -> PendingImport {
    let store = MemoryObjectStore::new();
    PendingImport {
        default_branch: "main".to_string(),
        head_oid: "1111111111111111111111111111111111111111".to_string(),
        tree_oid: "2222222222222222222222222222222222222222".to_string(),
        imported_at_unix: unix_now(),
        git_snapshot: source_blob("test git snapshot"),
        files: files
            .into_iter()
            .map(|(path, content)| PendingImportFile {
                path: path.to_string(),
                mode: "100644".to_string(),
                oid: format!("oid-{path}"),
                blob: put_source_blob(&store, TEST_REPO_ID, content.as_bytes()).unwrap(),
            })
            .collect(),
    }
}

fn source_blob(content: &str) -> crate::domain::store::SourceBlob {
    source_blob_from_bytes(content.as_bytes())
}

fn source_blob_from_bytes(bytes: &[u8]) -> crate::domain::store::SourceBlob {
    put_source_blob(
        AppState::test_state().object_store.as_ref(),
        TEST_REPO_ID,
        bytes,
    )
    .unwrap()
}

fn blob_content(blob: &crate::domain::store::SourceBlob) -> String {
    String::from_utf8(source_blob_bytes(&MemoryObjectStore::new(), blob).unwrap()).unwrap()
}

fn repo_with_readme() -> StoredRepository {
    let mut repo = test_repo(&test_owner_id());
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        parent_ids: Vec::new(),
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: "initial".to_string(),
        changes: vec![FileChange {
            visibility: Visibility::Public,
            path: ScopePath::parse("/README.md").unwrap(),
            old_content: None,
            new_content: Some(source_blob("hello")),
        }],
    });
    repo
}

fn receive_pack_update(changes: Vec<(&str, Option<&str>)>) -> ReceivePackUpdate {
    ReceivePackUpdate {
        branch: format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        head_oid: "1111111111111111111111111111111111111111".to_string(),
        base_git_snapshot_key: None,
        author_id: test_owner_id(),
        message: "owner push".to_string(),
        git_snapshot: source_blob("test staged git snapshot"),
        uploaded_blobs: Vec::new(),
        previous_config: None,
        config: repo_config(Visibility::Public),
        changes: changes
            .into_iter()
            .map(|(path, content)| ReceivePackFileChange {
                path: pending_scope_path(path).unwrap(),
                content: content.map(source_blob),
            })
            .collect(),
    }
}

fn repo_config(default_visibility: Visibility) -> RepoConfig {
    let default = match default_visibility {
        Visibility::Private => "private",
        Visibility::Public => "public",
    };
    RepoConfig::parse_json(
        format!(
            r#"{{
                "kind": "scope.repo-config",
                "version": 1,
                "visibility": {{
                    "default": "{default}",
                    "rules": []
                }}
            }}"#
        )
        .as_bytes(),
    )
    .unwrap()
}
