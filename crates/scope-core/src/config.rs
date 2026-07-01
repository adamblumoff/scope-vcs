use std::{
    path::{Path as FsPath, PathBuf},
    process::Command,
};

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
pub const REPOSITORY_INVITE_TOKEN_PREFIX: &str = "scope_invite_";
pub const CLI_BROWSER_LOGIN_ID_PREFIX: &str = "cli_browser_";
pub const CLI_BROWSER_LOGIN_SECRET_PREFIX: &str = "scope_browser_";
pub const CLI_CALLBACK_CODE_PREFIX: &str = "scope_callback_";
pub const CLI_EXCHANGE_GRANT_PREFIX: &str = "scope_otc_";
pub const CLI_SESSION_ID_PREFIX: &str = "cli_sess_";
pub const CLI_SESSION_TOKEN_PREFIX: &str = "scope_cli_";
pub const CLI_DEVICE_CODE_PREFIX: &str = "scope_device_";
pub const FIRST_PUSH_TOKEN_BYTES: usize = 32;
pub const CLI_BROWSER_LOGIN_TTL_SECS: u64 = 5 * 60;
pub const CLI_DEVICE_LOGIN_TTL_SECS: u64 = 10 * 60;
pub const CLI_EXCHANGE_GRANT_TTL_SECS: u64 = 5 * 60;
pub const CLI_SESSION_TTL_SECS: u64 = 30 * 24 * 60 * 60;
pub const CLI_DEVICE_LOGIN_POLL_INTERVAL_SECS: u64 = 2;
pub const RECEIVE_PACK_STAGING_BYTES: usize = 16;
pub const FIRST_PUSH_TOKEN_TTL_SECS: u64 = 5 * 60;
pub const EMPTY_GIT_OID: &str = "0000000000000000000000000000000000000000";
pub const GIT_UPLOAD_PACK: &str = "git-upload-pack";
pub const GIT_RECEIVE_PACK: &str = "git-receive-pack";
pub const DEFAULT_GIT_BRANCH: &str = "main";
pub const UNPUBLISHED_GIT_ERROR: &str = "repo is not published yet";
pub const MAX_RECEIVE_PACK_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_UPLOAD_PACK_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PENDING_IMPORT_FILES: usize = 10_000;
pub const MAX_PENDING_IMPORT_BLOB_BYTES: usize = 25 * 1024 * 1024;
pub const MAX_PENDING_IMPORT_TOTAL_BYTES: usize = 100 * 1024 * 1024;

pub fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
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
