use std::{
    path::{Path as FsPath, PathBuf},
    process::Command,
};

pub(crate) const SCOPE_APP_ORIGIN_ENV: &str = "SCOPE_APP_ORIGIN";
pub(crate) const SCOPE_REPO_ROOT_ENV: &str = "SCOPE_REPO_ROOT";
pub(crate) const SCOPE_STATE_PATH_ENV: &str = "SCOPE_STATE_PATH";
pub(crate) const SHOO_JWKS_URL: &str = "https://shoo.dev/.well-known/jwks.json";
pub(crate) const SHOO_ISSUER: &str = "https://shoo.dev";
pub(crate) const LOCAL_APP_ORIGIN: &str = "http://localhost:3000";
pub(crate) const FIRST_PUSH_TOKEN_PREFIX: &str = "scope_fp_";
pub(crate) const GIT_PUSH_TOKEN_PREFIX: &str = "scope_git_";
pub(crate) const FIRST_PUSH_TOKEN_BYTES: usize = 32;
pub(crate) const RECEIVE_PACK_STAGING_BYTES: usize = 16;
pub(crate) const FIRST_PUSH_TOKEN_TTL_SECS: u64 = 5 * 60;
pub(crate) const EMPTY_GIT_OID: &str = "0000000000000000000000000000000000000000";
pub(crate) const GIT_UPLOAD_PACK: &str = "git-upload-pack";
pub(crate) const GIT_RECEIVE_PACK: &str = "git-receive-pack";
pub(crate) const DEFAULT_GIT_BRANCH: &str = "main";
pub(crate) const UNPUBLISHED_GIT_ERROR: &str = "repo is not published yet";
pub(crate) const MAX_RECEIVE_PACK_BYTES: usize = 512 * 1024 * 1024;
pub(crate) const MAX_UPLOAD_PACK_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_PENDING_IMPORT_FILES: usize = 10_000;
pub(crate) const MAX_PENDING_IMPORT_BLOB_BYTES: usize = 25 * 1024 * 1024;
pub(crate) const MAX_PENDING_IMPORT_TOTAL_BYTES: usize = 100 * 1024 * 1024;

pub(crate) fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

pub(crate) fn state_path(repo_root: &FsPath) -> PathBuf {
    non_empty_env(SCOPE_STATE_PATH_ENV)
        .map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                repo_root.join(path)
            }
        })
        .unwrap_or_else(|| repo_root.join(".scope").join("state.json"))
}

pub(crate) fn git_repo_root() -> PathBuf {
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
