mod env;
mod file_object_store;

use crate::{
    AppState,
    auth::clerk::ClerkVerifier,
    config::{SCOPE_OPERATOR_TOKEN_ENV, data_dir, git_repo_root, non_empty_env},
    db::MetadataStore,
    domain::store::AppCatalog,
    object_store::EncryptedObjectStore,
    persistence::ensure_private_dir,
};
use std::sync::Arc;

pub use env::is_local_dev_env;

pub fn app_state_from_env() -> anyhow::Result<AppState> {
    let settings = env::validate_local_dev_environment()?;
    let repo_root = git_repo_root();
    let data_dir = data_dir(&repo_root);
    ensure_private_dir(&data_dir).map_err(|error| anyhow::anyhow!(error.message))?;

    let metadata = match settings.metadata_store {
        env::DevMetadataStore::Memory => MetadataStore::memory(AppCatalog::default()),
        env::DevMetadataStore::Postgres => MetadataStore::connect_from_env()?,
    };
    let object_store = Arc::new(EncryptedObjectStore::from_env(Arc::new(
        file_object_store::FileObjectStore::from_env(&data_dir),
    ))?);

    Ok(AppState {
        metadata,
        data_dir: Arc::new(data_dir),
        clerk: ClerkVerifier::from_env(),
        object_store,
        operator_token: non_empty_env(SCOPE_OPERATOR_TOKEN_ENV).map(Arc::from),
    })
}
