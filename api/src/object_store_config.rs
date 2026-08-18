use crate::config::{
    SCOPE_BUCKET_ACCESS_KEY_ID_ENV, SCOPE_BUCKET_ENDPOINT_ENV, SCOPE_BUCKET_FORCE_PATH_STYLE_ENV,
    SCOPE_BUCKET_NAME_ENV, SCOPE_BUCKET_REGION_ENV, SCOPE_BUCKET_SECRET_ACCESS_KEY_ENV,
    SCOPE_OBJECT_ENCRYPTION_KEY_ENV, non_empty_env,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
#[cfg(feature = "local-dev")]
use scope_object_store::{FileObjectStore, FileObjectStoreSettings};
use scope_object_store::{S3ObjectStore, S3ObjectStoreSettings};
#[cfg(feature = "local-dev")]
use std::path::{Path, PathBuf};

#[cfg(feature = "local-dev")]
const SCOPE_OBJECT_STORE_DIR_ENV: &str = "SCOPE_OBJECT_STORE_DIR";

pub(crate) fn encryption_key_from_env() -> anyhow::Result<[u8; 32]> {
    let encoded = required_env(SCOPE_OBJECT_ENCRYPTION_KEY_ENV)?;
    let decoded = BASE64.decode(encoded.trim()).map_err(|error| {
        anyhow::anyhow!("{SCOPE_OBJECT_ENCRYPTION_KEY_ENV} must be base64: {error}")
    })?;
    decoded.as_slice().try_into().map_err(|_| {
        anyhow::anyhow!("{SCOPE_OBJECT_ENCRYPTION_KEY_ENV} must decode to exactly 32 bytes")
    })
}

pub(crate) fn s3_from_env() -> anyhow::Result<S3ObjectStore> {
    S3ObjectStore::new(s3_settings_from_env()?).map_err(anyhow::Error::from)
}

pub(crate) fn s3_settings_from_env() -> anyhow::Result<S3ObjectStoreSettings> {
    let mut settings = S3ObjectStoreSettings::new(
        required_env(SCOPE_BUCKET_ENDPOINT_ENV)?,
        required_env(SCOPE_BUCKET_NAME_ENV)?,
        required_env(SCOPE_BUCKET_REGION_ENV)?,
        required_env(SCOPE_BUCKET_ACCESS_KEY_ID_ENV)?,
        required_env(SCOPE_BUCKET_SECRET_ACCESS_KEY_ENV)?,
    );
    settings.force_path_style = non_empty_env(SCOPE_BUCKET_FORCE_PATH_STYLE_ENV)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);
    Ok(settings)
}

#[cfg(feature = "local-dev")]
pub(crate) fn file_from_env(default_root: &Path) -> FileObjectStore {
    let root = non_empty_env(SCOPE_OBJECT_STORE_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| default_root.to_path_buf());
    FileObjectStore::new(FileObjectStoreSettings::new(root))
}

fn required_env(name: &str) -> anyhow::Result<String> {
    non_empty_env(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
}
