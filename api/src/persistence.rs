use crate::domain::store::{AppCatalog, CatalogError, StoredRepository, UserAccount};
use crate::{error::ApiError, state::AppState};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Write, path::Path as FsPath};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct PersistedState {
    pub(crate) users: BTreeMap<String, UserAccount>,
    pub(crate) repositories: BTreeMap<String, StoredRepository>,
}

#[cfg(test)]
pub(crate) fn test_state_path() -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("test clock must be after UNIX epoch")
        .as_nanos();
    std::env::temp_dir()
        .join(format!(
            "scope-vcs-test-state-{}-{nanos}",
            std::process::id()
        ))
        .join("state.json")
}

pub(crate) fn load_state(path: &FsPath) -> anyhow::Result<PersistedState> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PersistedState::default());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", path.display()));
        }
    };
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

pub(crate) fn persist_catalog(state: &AppState, catalog: &AppCatalog) -> Result<(), ApiError> {
    if let Some(parent) = state.state_path.parent() {
        ensure_private_dir(parent)?;
    }

    let bytes = serde_json::to_vec_pretty(&persisted_state_from_catalog(catalog))
        .map_err(ApiError::internal)?;
    let temp_path = state
        .state_path
        .with_extension(format!("json.{}.tmp", std::process::id()));
    {
        let mut file = fs::File::create(&temp_path).map_err(ApiError::internal)?;
        ensure_private_file(&file)?;
        file.write_all(&bytes).map_err(ApiError::internal)?;
        file.sync_all().map_err(ApiError::internal)?;
    }

    fs::rename(&temp_path, state.state_path.as_ref()).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        ApiError::internal(error)
    })?;

    Ok(())
}

fn ensure_private_file(_file: &fs::File) -> Result<(), ApiError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = _file.metadata().map_err(ApiError::internal)?.permissions();
        permissions.set_mode(0o600);
        _file
            .set_permissions(permissions)
            .map_err(ApiError::internal)?;
    }

    Ok(())
}

pub(crate) fn apply_persisted_state(catalog: &mut AppCatalog, state: &PersistedState) {
    catalog.users = state.users.clone();
    catalog.repositories = state.repositories.clone();
}

fn persisted_state_from_catalog(catalog: &AppCatalog) -> PersistedState {
    PersistedState {
        users: catalog.users.clone(),
        repositories: catalog.repositories.clone(),
    }
}
pub(crate) fn ensure_private_dir(path: &FsPath) -> Result<(), ApiError> {
    fs::create_dir_all(path).map_err(ApiError::internal)?;
    let metadata = fs::symlink_metadata(path).map_err(ApiError::internal)?;
    if !metadata.file_type().is_dir() {
        return Err(ApiError::internal_message(format!(
            "{} is not a directory",
            path.display()
        )));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = metadata.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).map_err(ApiError::internal)?;
        let mode = fs::symlink_metadata(path)
            .map_err(ApiError::internal)?
            .permissions()
            .mode()
            & 0o777;
        if mode != 0o700 {
            return Err(ApiError::internal_message(format!(
                "{} must be private to serve Git projections",
                path.display()
            )));
        }
    }

    Ok(())
}
pub(crate) fn unix_now() -> Result<u64, ApiError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(ApiError::internal)
}

pub(crate) fn catalog_error(error: CatalogError) -> ApiError {
    match error {
        CatalogError::InvalidRepositoryName(message) => ApiError::bad_request(message),
        CatalogError::RepositoryExists(repo) => {
            ApiError::conflict(format!("repo {repo} already exists"))
        }
    }
}
pub(crate) fn lock_catalog(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, AppCatalog>, ApiError> {
    state
        .catalog
        .lock()
        .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))
}
