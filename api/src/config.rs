use std::{
    path::{Path as FsPath, PathBuf},
    process::Command,
};

use scope_git::{DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES, GitStorageLimits};

pub const SCOPE_APP_ORIGIN_ENV: &str = "SCOPE_APP_ORIGIN";
pub const SCOPE_API_PUBLIC_URL_ENV: &str = "SCOPE_API_PUBLIC_URL";
pub const DATABASE_URL_ENV: &str = "DATABASE_URL";
pub const SCOPE_REPO_ROOT_ENV: &str = "SCOPE_REPO_ROOT";
pub const SCOPE_DATA_DIR_ENV: &str = "SCOPE_DATA_DIR";
pub const SCOPE_BUCKET_ENDPOINT_ENV: &str = "SCOPE_BUCKET_ENDPOINT";
pub const SCOPE_BUCKET_NAME_ENV: &str = "SCOPE_BUCKET_NAME";
pub const SCOPE_BUCKET_REGION_ENV: &str = "SCOPE_BUCKET_REGION";
pub const SCOPE_BUCKET_ACCESS_KEY_ID_ENV: &str = "SCOPE_BUCKET_ACCESS_KEY_ID";
pub const SCOPE_BUCKET_SECRET_ACCESS_KEY_ENV: &str = "SCOPE_BUCKET_SECRET_ACCESS_KEY";
pub const SCOPE_BUCKET_FORCE_PATH_STYLE_ENV: &str = "SCOPE_BUCKET_FORCE_PATH_STYLE";
pub const SCOPE_OBJECT_ENCRYPTION_KEY_ENV: &str = "SCOPE_OBJECT_ENCRYPTION_KEY";
pub const SCOPE_OBJECT_STORE_MAX_BYTES_ENV: &str = "SCOPE_OBJECT_STORE_MAX_BYTES";
pub const SCOPE_GIT_CACHE_MAX_BYTES_ENV: &str = "SCOPE_GIT_CACHE_MAX_BYTES";
pub const SCOPE_OPERATOR_TOKEN_ENV: &str = "SCOPE_OPERATOR_TOKEN";
pub const CLERK_ISSUER_ENV: &str = "CLERK_ISSUER";
pub const CLERK_JWKS_URL_ENV: &str = "CLERK_JWKS_URL";
pub const CLERK_AUTHORIZED_PARTIES_ENV: &str = "CLERK_AUTHORIZED_PARTIES";
pub const CLERK_AUDIENCE_ENV: &str = "CLERK_AUDIENCE";
pub const DEFAULT_CLERK_AUDIENCE: &str = "scope-api";
pub const LOCAL_APP_ORIGIN: &str = "http://localhost:3000";
pub const LOCAL_API_ORIGIN: &str = "http://localhost:8080";
pub const FIRST_PUSH_TOKEN_PREFIX: &str = "scope_fp_";
pub const GIT_PUSH_TOKEN_PREFIX: &str = "scope_git_";
pub const CLI_SESSION_TOKEN_PREFIX: &str = "scope_cli_";
pub const RECEIVE_PACK_STAGING_BYTES: usize = 16;
pub const EMPTY_GIT_OID: &str = "0000000000000000000000000000000000000000";
pub const GIT_UPLOAD_PACK: &str = "git-upload-pack";
pub const GIT_RECEIVE_PACK: &str = "git-receive-pack";
pub const DEFAULT_GIT_BRANCH: &str = "main";
pub const DEFAULT_GIT_CACHE_MAX_BYTES: usize = 10 * 1024 * 1024 * 1024;
pub const AWAITING_FIRST_PUSH_GIT_ERROR: &str = "repo is awaiting its first push";
pub const MAX_RECEIVE_PACK_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_UPLOAD_PACK_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PENDING_IMPORT_FILES: usize = 10_000;
pub const MAX_PENDING_IMPORT_BLOB_BYTES: usize = 25 * 1024 * 1024;

pub fn database_url_from_env() -> anyhow::Result<String> {
    non_empty_env(DATABASE_URL_ENV)
        .ok_or_else(|| anyhow::anyhow!("{DATABASE_URL_ENV} is required for Scope metadata storage"))
}

pub fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

pub fn default_git_storage_limits() -> GitStorageLimits {
    GitStorageLimits::default()
}

pub fn git_storage_limits_from_env() -> anyhow::Result<GitStorageLimits> {
    let max_object_bytes = parse_usize_env(
        SCOPE_OBJECT_STORE_MAX_BYTES_ENV,
        DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES,
    )?;
    GitStorageLimits::new(max_object_bytes).map_err(anyhow::Error::from)
}

pub fn git_cache_max_bytes_from_env() -> anyhow::Result<usize> {
    let bytes = parse_usize_env(SCOPE_GIT_CACHE_MAX_BYTES_ENV, DEFAULT_GIT_CACHE_MAX_BYTES)?;
    if bytes == 0 {
        anyhow::bail!("{SCOPE_GIT_CACHE_MAX_BYTES_ENV} must be greater than zero");
    }
    Ok(bytes)
}

fn parse_usize_env(name: &str, default: usize) -> anyhow::Result<usize> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .parse::<usize>()
            .map_err(|error| anyhow::anyhow!("{name} must be an integer: {error}")),
        _ => Ok(default),
    }
}

pub fn data_dir(repo_root: &FsPath) -> PathBuf {
    non_empty_env(SCOPE_DATA_DIR_ENV)
        .map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                repo_root.join(path)
            }
        })
        .unwrap_or_else(|| repo_root.join(".scope"))
}

pub fn git_repo_root() -> PathBuf {
    if let Some(root) = non_empty_env(SCOPE_REPO_ROOT_ENV) {
        return PathBuf::from(root);
    }

    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();
    if let Ok(output) = output
        && output.status.success()
        && let Ok(root) = String::from_utf8(output.stdout)
    {
        return PathBuf::from(root.trim());
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
