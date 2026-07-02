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
};
use axum::{Json, extract::State};
use std::sync::Arc;

pub use env::is_local_dev_env;

pub fn app_state_from_env() -> anyhow::Result<AppState> {
    let settings = env::validate_local_dev_environment()?;
    let repo_root = git_repo_root();
    let data_dir = data_dir(&repo_root);
    ensure_private_dir(&data_dir).map_err(|error| anyhow::anyhow!(error.message))?;

    let raw_object_store = Arc::new(EncryptedObjectStore::from_env(Arc::new(
        file_object_store::FileObjectStore::from_env(&data_dir),
    ))?);
    let metadata = match settings.metadata_store {
        env::DevMetadataStore::Memory => {
            let catalog = seed::catalog(raw_object_store.as_ref(), settings.seed_user)
                .map_err(|error| anyhow::anyhow!("seeding local dev catalog: {}", error.message))?;
            MetadataStore::memory(catalog)
        }
        env::DevMetadataStore::Postgres => MetadataStore::connect_from_env()?,
    };
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
    let grant = state.metadata.create_cli_exchange_grant(&user)?;
    let token = state.metadata.exchange_cli_grant(&grant.exchange_token)?;

    Ok(Json(CliSessionTokenResponse {
        session_token: token.session_token,
        expires_at_unix: token.expires_at_unix,
        identity: token.identity,
    }))
}
