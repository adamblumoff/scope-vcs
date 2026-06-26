use crate::domain::git_projection::build_virtual_git_projection;
use crate::domain::policy::{
    Policy, Principal, PrincipalKind, ScopePath, Visibility, VisibilityRule,
};
use crate::domain::projection::{
    AuthorVisibility, FileChange, FileVisibilityChange, LogicalCommit, MixedCommitPolicy,
    SourceGraph, project_graph,
};
use crate::domain::repo_actions::preview_publish_import;
use crate::domain::store::{
    AccountAccess, AppCatalog, FirstPushToken, GitPushToken, LineDiff, PendingImport,
    PendingImportFile, RepoMembership, RepoPublicationState, RepoRecord, RepoRole, RepoSettings,
    RepoStorageCleanup, StagedFileChange, StagedFileChangeKind, StagedRepoUpdate, StoredRepository,
    UserAccount,
};
use crate::{
    app::router,
    auth::{clerk::*, scope::*, tokens::*},
    config::*,
    git::{import::*, storage::*, upload::*, *},
    http::responses::*,
    object_store::{MemoryObjectStore, put_source_blob, source_blob_text},
    persistence::*,
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
mod commit_history;
mod device_login;
mod git_http;
mod git_receive;
mod obsolete_routes;
mod readiness;
mod repo_cleanup;
mod repo_lifecycle;
mod repo_visibility;
mod review_publish;

const TEST_CLERK_ISSUER: &str = "https://clerk.test";
const TEST_CLERK_AUDIENCE: &str = "scope-api";
const TEST_CLERK_USER_ID: &str = "user_owner";
const TEST_OWNER_EMAIL: &str = "owner@example.com";
const TEST_REPO_OWNER: &str = "owner";
const TEST_REPO_NAME: &str = "repo";
const TEST_REPO_ID: &str = "owner/repo";

fn reject_staged_update_as_owner(
    state: &AppState,
) -> Result<StagedRepoUpdate, crate::error::ApiError> {
    let rejected =
        state
            .metadata
            .reject_staged_update(TEST_REPO_OWNER, TEST_REPO_NAME, &test_owner_id())?;
    best_effort_drain_pending_source_blob_deletions(state);
    Ok(rejected)
}

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
        access: AccountAccess::Member,
    };
    let repo = test_repo(&owner_id);

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
        object_store: Arc::new(MemoryObjectStore::new()),
        operator_token: None,
    }
}

fn test_state_with_jwks() -> AppState {
    let state = AppState::test_state();
    cache_test_jwks(&state);
    state
}

fn test_state_with_metadata(metadata: crate::db::MetadataStore) -> AppState {
    let state = AppState {
        metadata,
        data_dir: Arc::new(test_data_dir()),
        clerk: ClerkVerifier::new_with_policy(
            Some(TEST_CLERK_ISSUER.to_string()),
            Some("http://127.0.0.1/.well-known/jwks.json".to_string()),
            test_clerk_policy(),
        ),
        object_store: Arc::new(MemoryObjectStore::new()),
        operator_token: None,
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

fn test_repo(owner_id: &str) -> StoredRepository {
    StoredRepository {
        record: RepoRecord {
            id: TEST_REPO_ID.to_string(),
            owner_handle: TEST_REPO_OWNER.to_string(),
            name: TEST_REPO_NAME.to_string(),
            owner_user_id: owner_id.to_string(),
            publication_state: RepoPublicationState::Published,
            default_visibility: Visibility::Public,
        },
        settings: RepoSettings::default(),
        first_push_token: None,
        git_push_token: None,
        git_clone_tokens: Vec::new(),
        pending_import: None,
        policy: Policy::new(Visibility::Public, owner_id),
        graph: SourceGraph {
            repo_id: TEST_REPO_ID.to_string(),
            commits: Vec::new(),
        },
        git_snapshot: None,
        staged_update: None,
        memberships: vec![RepoMembership {
            repo_id: TEST_REPO_ID.to_string(),
            user_id: owner_id.to_string(),
            role: RepoRole::Owner,
        }],
        invitations: Vec::new(),
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
    put_source_blob(
        AppState::test_state().object_store.as_ref(),
        TEST_REPO_ID,
        content.as_bytes(),
    )
    .unwrap()
}

fn blob_content(blob: &crate::domain::store::SourceBlob) -> String {
    source_blob_text(&MemoryObjectStore::new(), blob).unwrap()
}

fn repo_with_readme() -> StoredRepository {
    let mut repo = test_repo(&test_owner_id());
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        parent_ids: Vec::new(),
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: "initial".to_string(),
        mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
        changes: vec![FileChange {
            visibility: Visibility::Public,
            path: ScopePath::parse("/README.md").unwrap(),
            old_content: None,
            new_content: Some(source_blob("hello")),
        }],
        visibility_changes: Vec::new(),
    });
    repo
}

fn receive_pack_update(changes: Vec<(&str, Option<&str>)>) -> ReceivePackUpdate {
    ReceivePackUpdate {
        branch: format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        author_id: test_owner_id(),
        message: "owner push".to_string(),
        git_snapshot: source_blob("test staged git snapshot"),
        uploaded_blobs: Vec::new(),
        changes: changes
            .into_iter()
            .map(|(path, content)| ReceivePackFileChange {
                path: pending_scope_path(path).unwrap(),
                content: content.map(source_blob),
            })
            .collect(),
    }
}
