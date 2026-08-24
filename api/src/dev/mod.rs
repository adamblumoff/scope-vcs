mod env;

use crate::demo_seed as seed;
use crate::{
    AppState,
    auth::{clerk::ClerkVerifier, cli::CliAuthService},
    config::{SCOPE_OPERATOR_TOKEN_ENV, data_dir, git_repo_root, non_empty_env},
    error::ApiError,
    object_store_config::{encryption_key_from_env, file_from_env},
    persistence::{ensure_private_dir, unix_now},
    product_analytics::ProductAnalytics,
    push_intents::push_intent_signing_key,
    repo_events::RepoChangeBus,
    runtime_budgets::{BudgetedObjectStore, RuntimeBudgets},
};
use axum::{
    Json,
    extract::{Path, State},
};
use scope_api_contract::CliSessionTokenResponse;
use scope_object_store::{EncryptedObjectStore, ObjectStore};
use scope_postgres::db::MetadataStore;
use std::sync::Arc;

pub use env::is_local_dev_env;

pub fn local_maintenance_database_url() -> anyhow::Result<String> {
    Ok(env::validate_local_dev_environment()?.database_url)
}

pub async fn app_state_from_env() -> anyhow::Result<AppState> {
    let settings = env::validate_local_dev_environment()?;
    let repo_root = git_repo_root();
    let data_dir = data_dir(&repo_root);
    ensure_private_dir(&data_dir)
        .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
    let object_encryption_key = encryption_key_from_env()?;
    let push_intent_signing_key = push_intent_signing_key(&data_dir, Some(&object_encryption_key))
        .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;

    let raw_object_store = Arc::new(EncryptedObjectStore::new(
        Arc::new(file_from_env(&data_dir.join("objects"))),
        object_encryption_key,
    ));
    let catalog =
        seed::catalog(raw_object_store.as_ref(), settings.seed_user.clone()).map_err(|error| {
            anyhow::anyhow!(
                "building local dev catalog: {}",
                error.into_operator_diagnostic()
            )
        })?;
    let metadata = MetadataStore::connect(settings.database_url.clone()).await?;
    metadata
        .admin()
        .replace_catalog_for_seed(catalog)
        .await
        .map_err(|error| anyhow::anyhow!("seeding local dev database: {}", error.message))?;
    seed::seed_request_discussion_gallery(&metadata)
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "seeding local dev request discussions: {}",
                error.into_operator_diagnostic()
            )
        })?;
    seed::seed_run_gallery(
        &metadata,
        &settings.seed_user.handle,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| anyhow::anyhow!("reading the local dev clock: {error}"))?
            .as_secs(),
    )
    .await
    .map_err(|error| {
        anyhow::anyhow!(
            "seeding local dev runs: {}",
            error.into_operator_diagnostic()
        )
    })?;
    let repo_events = RepoChangeBus::default();
    let listener_bus = repo_events.clone();
    metadata
        .repositories()
        .start_repo_change_listener(move |payload| {
            listener_bus.publish_notification_payload(&payload)
        })?;
    let runtime_budgets = Arc::new(RuntimeBudgets::from_env()?);
    let object_store: Arc<dyn ObjectStore> = Arc::new(BudgetedObjectStore::new(
        raw_object_store,
        runtime_budgets.clone(),
    ));

    let repository_engine = crate::git::repository_engine::RepositoryEngine::new(
        data_dir.join("git-cache"),
        crate::config::git_cache_max_bytes_from_env()?,
    )
    .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
    let state = AppState {
        metadata,
        data_dir: Arc::new(data_dir),
        clerk: ClerkVerifier::from_env(),
        object_store,
        cache_grants: crate::cache_grants::CacheGrantIssuer::test(),
        runtime_budgets,
        operator_token: non_empty_env(SCOPE_OPERATOR_TOKEN_ENV).map(Arc::from),
        product_analytics: ProductAnalytics::disabled(),
        repo_events,
        push_intent_signing_key,
        repository_engine: repository_engine.clone(),
        #[cfg(test)]
        test_object_store: Arc::new(scope_object_store::MemoryObjectStore::new()),
    };
    state.backfill_repository_landing_files().await?;
    state.backfill_repository_workflow_catalogs().await?;
    repository_engine.start_reaper();
    state.start_run_attempt_recovery();
    state.start_run_retention();
    Ok(state)
}

pub(crate) async fn create_bench_cli_session(
    State(state): State<AppState>,
) -> Result<Json<CliSessionTokenResponse>, ApiError> {
    if !env::is_local_dev_env() {
        return Err(ApiError::not_found(
            "local dev benchmark auth is unavailable",
        ));
    }

    let settings = env::validate_local_dev_environment().map_err(|error| {
        ApiError::internal_message(format!("validating local dev benchmark auth: {error}"))
    })?;
    let user = seed::seed_user_account(settings.seed_user);
    mint_cli_session(&state, &user).await
}

pub(crate) async fn create_dev_cli_session(
    State(state): State<AppState>,
    Path(handle): Path<String>,
) -> Result<Json<CliSessionTokenResponse>, ApiError> {
    if !env::is_local_dev_env() {
        return Err(ApiError::not_found("local dev auth is unavailable"));
    }

    let settings = env::validate_local_dev_environment().map_err(|error| {
        ApiError::internal_message(format!("validating local dev auth: {error}"))
    })?;
    let user = seed::actor_account(settings.seed_user, &handle)
        .ok_or_else(|| ApiError::not_found("seeded local dev actor not found"))?;
    mint_cli_session(&state, &user).await
}

async fn mint_cli_session(
    state: &AppState,
    user: &scope_domain::store::UserAccount,
) -> Result<Json<CliSessionTokenResponse>, ApiError> {
    let now_unix = unix_now()?;
    let auth = CliAuthService::new(state.metadata.auth());
    let grant = auth.create_exchange_grant(user, now_unix).await?;
    let token = auth.exchange_grant(&grant.exchange_token, now_unix).await?;

    Ok(Json(CliSessionTokenResponse {
        session_token: token.session_token,
        expires_at_unix: token.expires_at_unix,
        identity: token.identity.into(),
    }))
}
