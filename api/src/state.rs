use crate::domain::policy::{Principal, ScopePath};
use crate::domain::projection_views::has_visible_projected_history;
use crate::domain::store::{
    AppCatalog, RepoPublicationState, RepositoryAccess, RepositoryActor, SourceBlob,
    StoredRepository, repo_id,
};
use crate::{
    auth::clerk::ClerkVerifier,
    config::{SCOPE_OPERATOR_TOKEN_ENV, data_dir, git_repo_root, non_empty_env},
    db::MetadataStore,
    error::ApiError,
    object_store::{EncryptedObjectStore, ObjectStore, S3ObjectStore},
    persistence::{ensure_private_dir, unix_now},
    repo_events::{RepoChangeBus, RepoChangeEvent},
    runtime_budgets::{BudgetedObjectStore, RuntimeBudgets},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

const PUSH_INTENT_TTL_SECS: u64 = 10 * 60;
const PUSH_INTENT_TOKEN_PREFIX: &str = "scope_pi_";
const PUSH_INTENT_KIND: &str = "scope.push-intent";
const PUSH_INTENT_VERSION: u8 = 1;
const PUSH_INTENT_SIGNING_KEY_ENV: &str = "SCOPE_PUSH_INTENT_SIGNING_KEY";
const PUSH_INTENT_SIGNING_KEY_FILE: &str = "push-intent-signing-key";
type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct AppState {
    pub(crate) metadata: MetadataStore,
    pub(crate) data_dir: Arc<PathBuf>,
    pub(crate) clerk: ClerkVerifier,
    pub(crate) object_store: Arc<dyn ObjectStore>,
    pub(crate) runtime_budgets: Arc<RuntimeBudgets>,
    pub(crate) operator_token: Option<Arc<str>>,
    pub(crate) repo_events: RepoChangeBus,
    pub(crate) push_intent_signing_key: Arc<[u8]>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PushIntentClaims {
    kind: String,
    version: u8,
    repo_id: String,
    user_id: String,
    head_oid: String,
    base_git_snapshot_key: Option<String>,
    expires_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ValidatedPushIntent {
    pub(crate) repo_id: String,
    pub(crate) user_id: String,
    pub(crate) head_oid: String,
    pub(crate) base_git_snapshot_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CreatedPushIntent {
    pub(crate) token: String,
    pub(crate) expires_at_unix: u64,
}

impl ValidatedPushIntent {
    pub(crate) fn ensure_repo_user(&self, repo_id: &str, user_id: &str) -> Result<(), ApiError> {
        if self.repo_id == repo_id && self.user_id == user_id {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                "Scope push intent does not match received Git push",
            ))
        }
    }

    pub(crate) fn base_for_head(&self, head_oid: &str) -> Result<Option<String>, ApiError> {
        if self.head_oid == head_oid {
            Ok(self.base_git_snapshot_key.clone())
        } else {
            Err(ApiError::forbidden(
                "Scope push intent does not match received Git push",
            ))
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct CleanupDrainReport {
    pub(crate) repo_storage: RepoStorageCleanupDrainReport,
    pub(crate) source_blobs: SourceBlobCleanupDrainReport,
}

impl CleanupDrainReport {
    pub(crate) fn has_failures(&self) -> bool {
        self.repo_storage.has_failures() || self.source_blobs.has_failures()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct RepoStorageCleanupDrainReport {
    pub(crate) attempted: usize,
    pub(crate) deleted: usize,
    pub(crate) retained: usize,
    pub(crate) failed: Vec<RepoStorageCleanupFailure>,
}

impl RepoStorageCleanupDrainReport {
    fn has_failures(&self) -> bool {
        !self.failed.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RepoStorageCleanupFailure {
    pub(crate) owner_handle: String,
    pub(crate) repo_name: String,
    pub(crate) error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct SourceBlobCleanupDrainReport {
    pub(crate) attempted: usize,
    pub(crate) deleted: usize,
    pub(crate) retained: usize,
    pub(crate) skipped_referenced: usize,
    pub(crate) failed_object_deletes: Vec<SourceBlobCleanupFailure>,
}

impl SourceBlobCleanupDrainReport {
    fn has_failures(&self) -> bool {
        !self.failed_object_deletes.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct SourceBlobCleanupFailure {
    pub(crate) object_key: String,
    pub(crate) sha256: String,
    pub(crate) git_oid: String,
    pub(crate) size_bytes: u64,
    pub(crate) error: String,
}

impl SourceBlobCleanupFailure {
    fn from_blob(blob: &SourceBlob, error: ApiError) -> Self {
        Self {
            object_key: blob.object_key.clone(),
            sha256: blob.sha256.clone(),
            git_oid: blob.git_oid.clone(),
            size_bytes: blob.size_bytes,
            error: error.message,
        }
    }
}

#[derive(Debug, Default)]
struct SourceBlobStorageDeleteReport {
    deleted_keys: BTreeSet<String>,
    failed: Vec<SourceBlobCleanupFailure>,
}

impl SourceBlobStorageDeleteReport {
    fn first_error(&self) -> Option<ApiError> {
        self.failed.first().map(|failure| {
            ApiError::service_unavailable(format!(
                "failed to clean source blob storage {}: {}",
                failure.object_key, failure.error
            ))
        })
    }
}

impl AppState {
    pub fn from_env() -> anyhow::Result<Self> {
        let repo_root = git_repo_root();
        let data_dir = data_dir(&repo_root);
        ensure_private_dir(&data_dir).map_err(|error| anyhow::anyhow!(error.message))?;
        let push_intent_signing_key =
            push_intent_signing_key(&data_dir).map_err(|error| anyhow::anyhow!(error.message))?;
        let metadata = MetadataStore::connect_from_env()?;
        let repo_events = RepoChangeBus::default();
        let runtime_budgets = Arc::new(RuntimeBudgets::from_env()?);
        let object_store = Arc::new(BudgetedObjectStore::new(
            Arc::new(EncryptedObjectStore::from_env(Arc::new(
                S3ObjectStore::from_env()?,
            ))?),
            runtime_budgets.clone(),
        ));
        metadata.start_repo_change_listener(repo_events.clone())?;

        let state = Self {
            metadata,
            data_dir: Arc::new(data_dir),
            clerk: ClerkVerifier::from_env(),
            object_store,
            runtime_budgets,
            operator_token: non_empty_env(SCOPE_OPERATOR_TOKEN_ENV).map(Arc::from),
            repo_events,
            push_intent_signing_key,
        };
        best_effort_drain_pending_repo_storage_deletions(&state);
        best_effort_drain_pending_source_blob_deletions(&state);
        Ok(state)
    }

    #[cfg(test)]
    pub(crate) fn test_state() -> Self {
        use crate::{domain::store::app_catalog, persistence::test_data_dir};

        let runtime_budgets = Arc::new(RuntimeBudgets::from_config(Default::default()));
        Self {
            metadata: MetadataStore::memory(app_catalog()),
            data_dir: Arc::new(test_data_dir()),
            clerk: ClerkVerifier::new_with_policy(
                Some("https://clerk.test".to_string()),
                Some("http://127.0.0.1/.well-known/jwks.json".to_string()),
                crate::auth::clerk::ClerkTokenPolicy {
                    authorized_parties: vec![crate::config::LOCAL_APP_ORIGIN.to_string()],
                    audiences: vec![crate::config::DEFAULT_CLERK_AUDIENCE.to_string()],
                },
            ),
            object_store: Arc::new(BudgetedObjectStore::new(
                Arc::new(crate::object_store::MemoryObjectStore::new()),
                runtime_budgets.clone(),
            )),
            runtime_budgets,
            operator_token: None,
            repo_events: RepoChangeBus::default(),
            push_intent_signing_key: Arc::from(b"scope-test-push-intent-signing-key".as_slice()),
        }
    }

    pub(crate) fn create_push_intent(
        &self,
        repo_id: &str,
        user_id: &str,
        head_oid: &str,
        base_git_snapshot_key: Option<String>,
    ) -> Result<CreatedPushIntent, ApiError> {
        let expires_at_unix = unix_now()?.saturating_add(PUSH_INTENT_TTL_SECS);
        let intent = PushIntentClaims {
            kind: PUSH_INTENT_KIND.to_string(),
            version: PUSH_INTENT_VERSION,
            repo_id: repo_id.to_string(),
            user_id: user_id.to_string(),
            head_oid: head_oid.to_string(),
            base_git_snapshot_key,
            expires_at_unix,
        };
        let token = encode_push_intent(&self.push_intent_signing_key, &intent)?;
        Ok(CreatedPushIntent {
            token,
            expires_at_unix,
        })
    }

    pub(crate) fn validate_push_intent_secret(
        &self,
        secret: &str,
    ) -> Result<ValidatedPushIntent, ApiError> {
        decode_push_intent(&self.push_intent_signing_key, secret).map(|intent| {
            ValidatedPushIntent {
                repo_id: intent.repo_id,
                user_id: intent.user_id,
                head_oid: intent.head_oid,
                base_git_snapshot_key: intent.base_git_snapshot_key,
            }
        })
    }

    pub(crate) fn git_cache_root(&self) -> Result<PathBuf, ApiError> {
        ensure_private_dir(&self.data_dir)?;
        let cache_root = self.data_dir.join("git-cache");
        ensure_private_dir(&cache_root)?;
        Ok(cache_root)
    }

    pub(crate) fn publish_repo_change(&self, repo_id: &str, version: u64, reason: &'static str) {
        let event = RepoChangeEvent::new(repo_id, version, reason);
        self.repo_events.publish_event(event.clone());
        if let Err(error) = self
            .metadata
            .notify_repo_change(self.repo_events.origin_id(), &event)
        {
            tracing::warn!(
                repo_id,
                version,
                reason,
                error = %error.message,
                "failed to publish repo change notification"
            );
        }
    }
}

pub(crate) fn push_intent_signing_key(data_dir: &Path) -> Result<Arc<[u8]>, ApiError> {
    if let Some(secret) = non_empty_env(PUSH_INTENT_SIGNING_KEY_ENV) {
        return Ok(Arc::from(secret.into_bytes()));
    }

    ensure_private_dir(data_dir)?;
    let key_path = data_dir.join(PUSH_INTENT_SIGNING_KEY_FILE);
    if key_path.exists() {
        let secret = fs::read_to_string(&key_path).map_err(ApiError::internal)?;
        let secret = secret.trim();
        if secret.is_empty() {
            return Err(ApiError::internal_message(
                "push intent signing key file is empty",
            ));
        }
        return Ok(Arc::from(secret.as_bytes()));
    }

    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!(
            "push intent signing key generation failed: {error}"
        ))
    })?;
    let secret = URL_SAFE_NO_PAD.encode(bytes);
    fs::write(&key_path, format!("{secret}\n")).map_err(ApiError::internal)?;
    Ok(Arc::from(secret.into_bytes()))
}

fn encode_push_intent(signing_key: &[u8], intent: &PushIntentClaims) -> Result<String, ApiError> {
    let payload = serde_json::to_vec(intent).map_err(ApiError::internal)?;
    let payload = URL_SAFE_NO_PAD.encode(payload);
    let signature = sign_push_intent(signing_key, payload.as_bytes())?;
    Ok(format!(
        "{PUSH_INTENT_TOKEN_PREFIX}{payload}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn decode_push_intent(signing_key: &[u8], token: &str) -> Result<PushIntentClaims, ApiError> {
    let Some(token) = token.trim().strip_prefix(PUSH_INTENT_TOKEN_PREFIX) else {
        return Err(ApiError::forbidden("valid Scope push intent required"));
    };
    let Some((payload, signature)) = token.split_once('.') else {
        return Err(ApiError::forbidden("valid Scope push intent required"));
    };
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| ApiError::forbidden("valid Scope push intent required"))?;
    verify_push_intent_signature(signing_key, payload.as_bytes(), &signature)?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| ApiError::forbidden("valid Scope push intent required"))?;
    let intent: PushIntentClaims = serde_json::from_slice(&payload)
        .map_err(|_| ApiError::forbidden("valid Scope push intent required"))?;
    if intent.kind != PUSH_INTENT_KIND || intent.version != PUSH_INTENT_VERSION {
        return Err(ApiError::forbidden("valid Scope push intent required"));
    }
    if intent.expires_at_unix <= unix_now()? {
        return Err(ApiError::forbidden("valid Scope push intent required"));
    }
    Ok(intent)
}

fn sign_push_intent(signing_key: &[u8], payload: &[u8]) -> Result<Vec<u8>, ApiError> {
    let mut mac = HmacSha256::new_from_slice(signing_key).map_err(ApiError::internal)?;
    mac.update(payload);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn verify_push_intent_signature(
    signing_key: &[u8],
    payload: &[u8],
    signature: &[u8],
) -> Result<(), ApiError> {
    let mut mac = HmacSha256::new_from_slice(signing_key).map_err(ApiError::internal)?;
    mac.update(payload);
    mac.verify_slice(signature)
        .map_err(|_| ApiError::forbidden("valid Scope push intent required"))
}

pub(crate) fn find_repo(
    state: &AppState,
    owner: &str,
    name: &str,
) -> Result<StoredRepository, ApiError> {
    state
        .metadata
        .repository(owner, name)?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))
}

pub(crate) fn ensure_repo_read(
    state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<(), ApiError> {
    if can_read_path(state, repo, principal, &ScopePath::root())?
        || (repo.record.publication_state == RepoPublicationState::Published
            && has_visible_projected_history(repo, principal))
    {
        Ok(())
    } else {
        Err(ApiError::not_found(format!(
            "repo {} not found",
            repo.record.id
        )))
    }
}

pub(crate) fn access_for_principal(
    _state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<RepositoryAccess, ApiError> {
    Ok(repo.access_for_principal(principal))
}

pub(crate) fn can_read_path(
    _state: &AppState,
    repo: &StoredRepository,
    principal: &Principal,
    path: &ScopePath,
) -> Result<bool, ApiError> {
    if principal.kind == crate::domain::policy::PrincipalKind::Public {
        return Ok(
            repo.record.publication_state == RepoPublicationState::Published
                && repo.policy.can_read(path, false),
        );
    }

    let access = access_for_principal(_state, repo, principal)?;
    Ok(match access.actor {
        RepositoryActor::Owner => repo.policy.can_read(path, true),
        RepositoryActor::Member => {
            repo.record.publication_state == RepoPublicationState::Published
                && repo.policy.can_read(path, access.can_read_private_files)
        }
        RepositoryActor::Public => false,
    })
}

pub(crate) fn repo_source_blobs(repo: &StoredRepository) -> Vec<SourceBlob> {
    repo.source_blobs()
}

pub(crate) fn delete_unreferenced_source_blobs(
    state: &AppState,
    blobs: &[SourceBlob],
) -> Result<(), ApiError> {
    let blobs = blobs.to_vec();
    let blobs = state
        .metadata
        .read(move |catalog| Ok(unreferenced_source_blobs(catalog, &blobs)))?;
    let report = delete_source_blob_storage(state, &blobs);
    match report.first_error() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn unreferenced_source_blobs(catalog: &AppCatalog, blobs: &[SourceBlob]) -> Vec<SourceBlob> {
    let referenced = catalog
        .repositories
        .values()
        .flat_map(repo_source_blobs)
        .map(|blob| blob.object_key)
        .collect::<std::collections::BTreeSet<_>>();
    unreferenced_source_blobs_by_key(&referenced, blobs)
}

fn unreferenced_source_blobs_by_key(
    referenced: &std::collections::BTreeSet<String>,
    blobs: &[SourceBlob],
) -> Vec<SourceBlob> {
    let mut unreferenced = std::collections::BTreeMap::new();
    for blob in blobs {
        if !referenced.contains(blob.object_key.as_str()) {
            unreferenced.entry(blob.object_key.as_str()).or_insert(blob);
        }
    }
    unreferenced.values().cloned().cloned().collect()
}

fn delete_source_blob_storage(
    state: &AppState,
    blobs: &[SourceBlob],
) -> SourceBlobStorageDeleteReport {
    let mut report = SourceBlobStorageDeleteReport::default();
    for blob in blobs {
        match delete_source_blob_storage_entry(state, blob) {
            Ok(()) => {
                report.deleted_keys.insert(blob.object_key.clone());
            }
            Err(error) => {
                tracing::warn!(
                    ?error,
                    object_key = %blob.object_key,
                    "failed to clean source blob storage"
                );
                report
                    .failed
                    .push(SourceBlobCleanupFailure::from_blob(blob, error));
            }
        }
    }
    report
}

fn delete_source_blob_storage_entry(state: &AppState, blob: &SourceBlob) -> Result<(), ApiError> {
    crate::git::storage::delete_raw_git_snapshot_cache(state, blob)?;
    state.object_store.delete(&blob.object_key)
}

pub(crate) fn drain_pending_cleanup(state: &AppState) -> Result<CleanupDrainReport, ApiError> {
    Ok(CleanupDrainReport {
        repo_storage: drain_pending_repo_storage_deletions_report(state)?,
        source_blobs: drain_pending_source_blob_deletions_report(state)?,
    })
}

pub(crate) fn drain_pending_repo_storage_deletions_report(
    state: &AppState,
) -> Result<RepoStorageCleanupDrainReport, ApiError> {
    let metadata = state.metadata.clone();
    let state = state.clone();
    metadata.update_pending_repo_storage_deletions(move |pending, live_repo_ids| {
        let mut report = RepoStorageCleanupDrainReport::default();
        let mut retained = Vec::new();
        for cleanup in pending {
            if live_repo_ids.contains(&repo_id(&cleanup.owner_handle, &cleanup.repo_name)) {
                retained.push(cleanup);
                continue;
            }

            report.attempted += 1;
            match crate::git::storage::delete_repo_storage(
                &state,
                &cleanup.owner_handle,
                &cleanup.repo_name,
            ) {
                Ok(()) => {
                    report.deleted += 1;
                }
                Err(error) => {
                    tracing::warn!(
                        ?error,
                        owner = %cleanup.owner_handle,
                        repo = %cleanup.repo_name,
                        "failed to clean deleted repo filesystem storage"
                    );
                    report.failed.push(RepoStorageCleanupFailure {
                        owner_handle: cleanup.owner_handle.clone(),
                        repo_name: cleanup.repo_name.clone(),
                        error: error.message,
                    });
                    retained.push(cleanup);
                }
            }
        }
        report.retained = retained.len();
        Ok((report, retained))
    })
}

pub(crate) fn drain_pending_repo_storage_deletions(state: &AppState) -> Result<(), ApiError> {
    let report = drain_pending_repo_storage_deletions_report(state)?;
    match report.failed.first() {
        Some(failure) => Err(ApiError::service_unavailable(format!(
            "failed to clean deleted repo storage {}/{}: {}",
            failure.owner_handle, failure.repo_name, failure.error
        ))),
        None => Ok(()),
    }
}

pub(crate) fn best_effort_drain_pending_repo_storage_deletions(state: &AppState) {
    if let Err(error) = drain_pending_repo_storage_deletions(state) {
        tracing::warn!(?error, "failed to drain pending repo storage deletions");
    }
}

pub(crate) fn persist_pending_source_blob_deletions(
    state: &AppState,
    blobs: &[SourceBlob],
) -> Result<(), ApiError> {
    if blobs.is_empty() {
        return Ok(());
    }
    let blobs = blobs.to_vec();
    state.metadata.queue_pending_source_blob_deletions(blobs)
}

pub(crate) fn best_effort_cleanup_rollback_source_blobs(state: &AppState, blobs: &[SourceBlob]) {
    if blobs.is_empty() {
        return;
    }
    match persist_pending_source_blob_deletions(state, blobs) {
        Ok(()) => best_effort_drain_pending_source_blob_deletions(state),
        Err(queue_error) => {
            tracing::warn!(?queue_error, "failed to queue rollback source blob cleanup");
            if let Err(delete_error) = delete_unreferenced_source_blobs(state, blobs) {
                tracing::warn!(
                    ?delete_error,
                    "failed to delete rollback source blobs without queued retry"
                );
            }
        }
    }
}

pub(crate) fn drain_pending_source_blob_deletions_report(
    state: &AppState,
) -> Result<SourceBlobCleanupDrainReport, ApiError> {
    let metadata = state.metadata.clone();
    let state = state.clone();
    metadata.update_pending_source_blob_deletions(move |pending, referenced_blob_keys| {
        let mut report = SourceBlobCleanupDrainReport::default();
        if pending.is_empty() {
            return Ok((report, pending));
        }

        let unreferenced = unreferenced_source_blobs_by_key(referenced_blob_keys, &pending);
        report.skipped_referenced = pending.len().saturating_sub(unreferenced.len());
        report.attempted = unreferenced.len();
        let delete_report = delete_source_blob_storage(&state, &unreferenced);
        report.deleted = delete_report.deleted_keys.len();
        report.failed_object_deletes = delete_report.failed;
        let retained = unreferenced
            .into_iter()
            .filter(|blob| !delete_report.deleted_keys.contains(&blob.object_key))
            .collect::<Vec<_>>();
        report.retained = retained.len();
        Ok((report, retained))
    })
}

pub(crate) fn drain_pending_source_blob_deletions(state: &AppState) -> Result<(), ApiError> {
    let report = drain_pending_source_blob_deletions_report(state)?;
    match report.failed_object_deletes.first().map(|failure| {
        ApiError::service_unavailable(format!(
            "failed to clean source blob storage {}: {}",
            failure.object_key, failure.error
        ))
    }) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub(crate) fn best_effort_drain_pending_source_blob_deletions(state: &AppState) {
    if let Err(error) = drain_pending_source_blob_deletions(state) {
        tracing::warn!(?error, "failed to drain pending source blob deletions");
    }
}
