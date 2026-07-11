mod env;
mod file_object_store;
mod seed;

use crate::{
    AppState,
    auth::clerk::ClerkVerifier,
    config::{SCOPE_OPERATOR_TOKEN_ENV, data_dir, git_repo_root, non_empty_env},
    db::MetadataStore,
    error::ApiError,
    http::responses::CliSessionTokenResponse,
    object_store::{EncryptedObjectStore, ObjectStore},
    persistence::ensure_private_dir,
    repo_events::RepoChangeBus,
    runtime_budgets::{BudgetedObjectStore, RuntimeBudgets},
    state::push_intent_signing_key,
};
use axum::{Json, extract::State};
use std::sync::Arc;

pub use env::is_local_dev_env;

pub async fn app_state_from_env() -> anyhow::Result<AppState> {
    let settings = env::validate_local_dev_environment()?;
    let repo_root = git_repo_root();
    let data_dir = data_dir(&repo_root);
    ensure_private_dir(&data_dir).map_err(|error| anyhow::anyhow!(error.message))?;
    let push_intent_signing_key = push_intent_signing_key(&data_dir)
        .map_err(|error| anyhow::anyhow!(error.into_message()))?;

    let raw_object_store = Arc::new(EncryptedObjectStore::from_env(Arc::new(
        file_object_store::FileObjectStore::from_env(&data_dir),
    ))?);
    let catalog = seed::catalog(raw_object_store.as_ref(), settings.seed_user)
        .map_err(|error| anyhow::anyhow!("building local dev catalog: {}", error.message()))?;
    let metadata = MetadataStore::connect_from_env().await?;
    metadata
        .replace_catalog_for_local_dev(catalog)
        .await
        .map_err(|error| anyhow::anyhow!("seeding local dev database: {}", error.message))?;
    let repo_events = RepoChangeBus::default();
    metadata.start_repo_change_listener(repo_events.clone())?;
    let runtime_budgets = Arc::new(RuntimeBudgets::from_env()?);
    let object_store: Arc<dyn ObjectStore> = Arc::new(BudgetedObjectStore::new(
        raw_object_store,
        runtime_budgets.clone(),
    ));

    Ok(AppState {
        metadata,
        data_dir: Arc::new(data_dir),
        clerk: ClerkVerifier::from_env(),
        object_store,
        runtime_budgets,
        operator_token: non_empty_env(SCOPE_OPERATOR_TOKEN_ENV).map(Arc::from),
        repo_events,
        push_intent_signing_key,
        #[cfg(test)]
        test_object_store: Arc::new(crate::object_store::MemoryObjectStore::new()),
    })
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
    let grant = state.metadata.create_cli_exchange_grant(&user).await?;
    let token = state
        .metadata
        .exchange_cli_grant(&grant.exchange_token)
        .await?;

    Ok(Json(CliSessionTokenResponse {
        session_token: token.session_token,
        expires_at_unix: token.expires_at_unix,
        identity: token.identity,
    }))
}
