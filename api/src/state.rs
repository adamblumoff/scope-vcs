use crate::{
    auth::clerk::ClerkVerifier,
    cache_grants::CacheGrantIssuer,
    config::{
        SCOPE_OPERATOR_TOKEN_ENV, data_dir, database_url_from_env, git_cache_max_bytes_from_env,
        git_public_url_from_env, git_repo_root, non_empty_env,
    },
    git::repository_engine::RepositoryEngine,
    object_store_config::{encryption_key_from_env, git_segment_store_from_env, s3_from_env},
    persistence::ensure_private_dir,
    product_analytics::ProductAnalytics,
    push_intents::push_intent_signing_key,
    repo_events::RepoChangeBus,
    runtime_budgets::{BudgetedObjectStore, RuntimeBudgets},
    use_cases::content_cleanup::best_effort_drain_pending_repo_storage_deletions,
};
use scope_domain::repository::git::GitSegmentUploadState;
use scope_git_storage::GitSegmentStore;
#[cfg(any(test, feature = "test-support"))]
use scope_git_storage::{GitSegmentStoreConfig, MemoryMultipartStore, SegmentEncryptionKey};
use scope_object_store::{EncryptedObjectStore, ObjectStore};
use scope_postgres::db::MetadataStore;
use std::{path::PathBuf, sync::Arc, time::Duration};

#[derive(Clone)]
pub struct AppState {
    pub(crate) metadata: MetadataStore,
    pub(crate) data_dir: Arc<PathBuf>,
    pub(crate) clerk: ClerkVerifier,
    pub(crate) object_store: Arc<dyn ObjectStore>,
    pub(crate) git_segment_store: Arc<GitSegmentStore>,
    pub(crate) cache_grants: CacheGrantIssuer,
    pub(crate) runtime_budgets: Arc<RuntimeBudgets>,
    pub(crate) operator_token: Option<Arc<str>>,
    pub(crate) product_analytics: ProductAnalytics,
    pub(crate) repo_events: RepoChangeBus,
    pub(crate) push_intent_signing_key: Arc<[u8]>,
    pub(crate) repository_engine: Arc<RepositoryEngine>,
    pub(crate) git_public_url: Arc<str>,
    #[cfg(test)]
    pub(crate) test_object_store: Arc<scope_object_store::MemoryObjectStore>,
}

impl AppState {
    pub async fn from_env() -> anyhow::Result<Self> {
        let git_public_url = git_public_url_from_env(None)?;
        let repo_root = git_repo_root();
        let data_dir = data_dir(&repo_root);
        ensure_private_dir(&data_dir)
            .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
        let object_encryption_key = encryption_key_from_env()?;
        let git_segment_store = Arc::new(git_segment_store_from_env(
            data_dir.join("git-segments"),
            object_encryption_key,
        )?);
        git_segment_store.cleanup_all_local().await?;
        let push_intent_signing_key =
            push_intent_signing_key(&data_dir, Some(&object_encryption_key))
                .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
        let metadata = MetadataStore::connect(database_url_from_env()?).await?;
        let repo_events = RepoChangeBus::default();
        let runtime_budgets = Arc::new(RuntimeBudgets::from_env()?);
        let s3 = tokio::task::spawn_blocking(s3_from_env).await??;
        let object_store = Arc::new(BudgetedObjectStore::new(
            Arc::new(EncryptedObjectStore::new(
                Arc::new(s3),
                object_encryption_key,
            )),
            runtime_budgets.clone(),
        ));
        let cache_grants = CacheGrantIssuer::from_env()?;
        let listener_bus = repo_events.clone();
        metadata
            .repositories()
            .start_repo_change_listener(move |payload| {
                listener_bus.publish_notification_payload(&payload)
            })?;
        let repository_engine =
            RepositoryEngine::new(data_dir.join("git-cache"), git_cache_max_bytes_from_env()?)
                .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
        let product_analytics = ProductAnalytics::from_env().await?;

        let state = Self {
            metadata,
            data_dir: Arc::new(data_dir),
            clerk: ClerkVerifier::from_env(),
            object_store,
            git_segment_store,
            cache_grants,
            runtime_budgets,
            operator_token: non_empty_env(SCOPE_OPERATOR_TOKEN_ENV).map(Arc::from),
            product_analytics,
            repo_events,
            push_intent_signing_key,
            repository_engine: repository_engine.clone(),
            git_public_url: Arc::from(git_public_url),
            #[cfg(test)]
            test_object_store: Arc::new(scope_object_store::MemoryObjectStore::new()),
        };
        repository_engine.start_reaper();
        state.start_run_attempt_recovery();
        state.start_run_retention();
        state.start_git_segment_recovery();
        best_effort_drain_pending_repo_storage_deletions(&state).await;
        Ok(state)
    }

    pub async fn shutdown_product_analytics(&self) {
        self.product_analytics.shutdown().await;
    }

    pub(crate) fn start_git_segment_recovery(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10 * 60));
            loop {
                interval.tick().await;
                if let Err(error) = state.recover_stale_git_segments().await {
                    tracing::warn!(
                        error = %error.into_operator_diagnostic(),
                        "stale Git segment recovery failed"
                    );
                }
            }
        });
    }

    async fn recover_stale_git_segments(&self) -> Result<(), crate::error::ApiError> {
        let now = crate::persistence::unix_now()?;
        let cutoff = now.saturating_sub(15 * 60);
        let uploads = self
            .metadata
            .repositories()
            .load_stale_git_segment_uploads(cutoff, 100)
            .await?;
        let orphan_count = uploads.len();
        for upload in uploads {
            let may_delete = match upload.state {
                GitSegmentUploadState::Uploading | GitSegmentUploadState::Ready => {
                    self.metadata
                        .repositories()
                        .abandon_git_segment_upload(&upload.segment_id, now)
                        .await?
                }
                GitSegmentUploadState::Deleting => true,
                GitSegmentUploadState::Published
                | GitSegmentUploadState::Retained
                | GitSegmentUploadState::Deleted => false,
            };
            if !may_delete {
                continue;
            }
            if let Err(error) = self
                .git_segment_store
                .cleanup_remote_bounded(&upload.object_key)
                .await
            {
                tracing::warn!(
                    repository_id = upload.repository_id,
                    segment_id = upload.segment_id,
                    error = %error,
                    "stale Git segment remote cleanup failed"
                );
                continue;
            }
            if let Err(error) = self
                .git_segment_store
                .cleanup_local(&upload.repository_id, &upload.segment_id)
                .await
            {
                tracing::warn!(
                    repository_id = upload.repository_id,
                    segment_id = upload.segment_id,
                    error = %error,
                    "stale Git segment local cleanup failed"
                );
            }
            self.metadata
                .repositories()
                .mark_git_segment_upload_deleted(
                    &upload.segment_id,
                    crate::persistence::unix_now()?,
                )
                .await?;
        }
        tracing::info!(
            phase = "recovery",
            repository_id = "all",
            segment_id = "all",
            success = true,
            duration_us = 0_u64,
            bytes = 0_u64,
            blocked_us = 0_u64,
            active_ingests = 0_u64,
            buffered_bytes = 0_u64,
            disk_free_bytes = 0_u64,
            ledger_uploading = 0_u64,
            ledger_ready = 0_u64,
            ledger_published = 0_u64,
            orphan_count,
            "Git segment ingest telemetry"
        );
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn test_state() -> Self {
        use crate::persistence::test_data_dir;

        let data_dir = test_data_dir();
        let runtime_budgets = Arc::new(RuntimeBudgets::from_config(Default::default()));
        let test_object_store = Arc::new(scope_object_store::MemoryObjectStore::new());
        let git_segment_store = Arc::new(
            GitSegmentStore::new(
                Arc::new(MemoryMultipartStore::default()),
                SegmentEncryptionKey::new("test", [9_u8; 32]).unwrap(),
                GitSegmentStoreConfig::new(data_dir.join("git-segments")),
            )
            .unwrap(),
        );
        let target = scope_postgres::db::TestDatabaseTarget::required().unwrap();
        let metadata = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        Self {
            metadata,
            data_dir: Arc::new(data_dir.clone()),
            clerk: ClerkVerifier::new_with_policy(
                Some("https://clerk.test".to_string()),
                Some("http://127.0.0.1/.well-known/jwks.json".to_string()),
                crate::auth::clerk::ClerkTokenPolicy {
                    authorized_parties: vec![crate::config::LOCAL_APP_ORIGIN.to_string()],
                    audiences: vec![crate::config::DEFAULT_CLERK_AUDIENCE.to_string()],
                },
            ),
            object_store: Arc::new(BudgetedObjectStore::new(
                test_object_store.clone(),
                runtime_budgets.clone(),
            )),
            git_segment_store,
            cache_grants: CacheGrantIssuer::test(),
            runtime_budgets,
            operator_token: None,
            product_analytics: ProductAnalytics::disabled(),
            repo_events: RepoChangeBus::default(),
            push_intent_signing_key: Arc::from(b"scope-test-push-intent-signing-key".as_slice()),
            repository_engine: RepositoryEngine::new(
                data_dir.join("git-cache"),
                crate::config::DEFAULT_GIT_CACHE_MAX_BYTES,
            )
            .unwrap(),
            git_public_url: Arc::from(crate::config::LOCAL_API_ORIGIN),
            #[cfg(test)]
            test_object_store,
        }
    }
}
