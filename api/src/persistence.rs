use crate::domain::store::CatalogError;
use crate::error::ApiError;
use std::{fs, path::Path as FsPath};

#[cfg(test)]
pub(crate) fn test_data_dir() -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("test clock must be after UNIX epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "scope-vcs-test-data-{}-{nanos}",
        std::process::id()
    ))
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
#[cfg(test)]
pub(crate) fn lock_catalog(
    state: &crate::state::AppState,
) -> Result<std::sync::MutexGuard<'_, crate::domain::store::AppCatalog>, ApiError> {
    state.metadata.test_catalog()
}
