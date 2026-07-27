use crate::{
    auth::clerk::ClerkVerifier,
    config::{
        SCOPE_OPERATOR_TOKEN_ENV, data_dir, database_url_from_env, deploy_revision, git_repo_root,
        non_empty_env,
    },
    git::cache::RawGitCacheRegistry,
    object_store_config::{encryption_key_from_env, s3_from_env},
    persistence::ensure_private_dir,
    push_intents::push_intent_signing_key,
    repo_cleanup::best_effort_drain_pending_repo_storage_deletions,
    repo_events::RepoChangeBus,
    runtime_budgets::{BudgetedObjectStore, RuntimeBudgets},
};
use scope_object_store::{EncryptedObjectStore, ObjectStore};
use scope_postgres::db::MetadataStore;
use std::{path::PathBuf, sync::Arc};

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
    pub(crate) raw_git_cache: Arc<RawGitCacheRegistry>,
    #[cfg(test)]
    pub(crate) test_object_store: Arc<scope_object_store::MemoryObjectStore>,
}

impl AppState {
    pub async fn from_env() -> anyhow::Result<Self> {
        let repo_root = git_repo_root();
        let data_dir = data_dir(&repo_root);
        ensure_private_dir(&data_dir).map_err(|error| anyhow::anyhow!(error.into_message()))?;
        let push_intent_signing_key = push_intent_signing_key(&data_dir)
            .map_err(|error| anyhow::anyhow!(error.into_message()))?;
        let metadata = MetadataStore::connect(database_url_from_env()?, deploy_revision()).await?;
        let repo_events = RepoChangeBus::default();
        let runtime_budgets = Arc::new(RuntimeBudgets::from_env()?);
        let object_store = Arc::new(BudgetedObjectStore::new(
            Arc::new(EncryptedObjectStore::new(
                Arc::new(s3_from_env()?),
                encryption_key_from_env()?,
            )),
            runtime_budgets.clone(),
        ));
        let listener_bus = repo_events.clone();
        metadata
            .repositories()
            .start_repo_change_listener(move |payload| {
                listener_bus.publish_notification_payload(&payload)
            })?;
        let raw_git_cache = RawGitCacheRegistry::new(data_dir.join("git-cache"))
            .map_err(|error| anyhow::anyhow!(error.into_message()))?;

        let state = Self {
            metadata,
            data_dir: Arc::new(data_dir),
            clerk: ClerkVerifier::from_env(),
            object_store,
            runtime_budgets,
            operator_token: non_empty_env(SCOPE_OPERATOR_TOKEN_ENV).map(Arc::from),
            repo_events,
            push_intent_signing_key,
            raw_git_cache: raw_git_cache.clone(),
            #[cfg(test)]
            test_object_store: Arc::new(scope_object_store::MemoryObjectStore::new()),
        };
        state.start_raw_git_cache_reaper();
        best_effort_drain_pending_repo_storage_deletions(&state).await;
        Ok(state)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn test_state() -> Self {
        use crate::persistence::test_data_dir;

        let data_dir = test_data_dir();
        let runtime_budgets = Arc::new(RuntimeBudgets::from_config(Default::default()));
        let test_object_store = Arc::new(scope_object_store::MemoryObjectStore::new());
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
            runtime_budgets,
            operator_token: None,
            repo_events: RepoChangeBus::default(),
            push_intent_signing_key: Arc::from(b"scope-test-push-intent-signing-key".as_slice()),
            raw_git_cache: RawGitCacheRegistry::new(data_dir.join("git-cache")).unwrap(),
            #[cfg(test)]
            test_object_store,
        }
    }
}
