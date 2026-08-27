use crate::config::{
    SCOPE_BUCKET_ACCESS_KEY_ID_ENV, SCOPE_BUCKET_ENDPOINT_ENV, SCOPE_BUCKET_FORCE_PATH_STYLE_ENV,
    SCOPE_BUCKET_NAME_ENV, SCOPE_BUCKET_REGION_ENV, SCOPE_BUCKET_SECRET_ACCESS_KEY_ENV,
    SCOPE_OBJECT_ENCRYPTION_KEY_ENV, non_empty_env,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scope_git_storage::{
    GitSegmentStore, S3MultipartSettings, S3MultipartStore, SegmentEncryptionKey,
};
#[cfg(feature = "local-dev")]
use scope_object_store::{FileObjectStore, FileObjectStoreSettings};
use scope_object_store::{S3ObjectStore, S3ObjectStoreSettings};
#[cfg(feature = "local-dev")]
use std::path::Path;
use std::path::PathBuf;

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

pub(crate) fn git_segment_store_from_env(
    local_root: PathBuf,
    encryption_key: [u8; 32],
) -> anyhow::Result<GitSegmentStore> {
    let s3 = s3_settings_from_env()?;
    let backend = S3MultipartStore::new(S3MultipartSettings {
        endpoint: s3.endpoint,
        bucket: s3.bucket,
        region: s3.region,
        access_key_id: s3.access_key_id,
        secret_access_key: s3.secret_access_key,
        force_path_style: s3.force_path_style,
    })?;
    let key = SegmentEncryptionKey::new("primary", encryption_key)?;
    GitSegmentStore::new(
        std::sync::Arc::new(backend),
        key,
        crate::config::git_segment_store_config_from_env(local_root)?,
    )
    .map_err(anyhow::Error::from)
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
