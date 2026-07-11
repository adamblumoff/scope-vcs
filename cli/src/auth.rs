use crate::api::{AuthenticatedSession, validate_session_token};
use anyhow::Context;
#[cfg(any(target_os = "macos", windows))]
use keyring_core::{Entry, Error as KeyringError};
use reqwest::blocking::Client;
#[cfg(any(target_os = "macos", windows))]
use std::sync::Once;
#[cfg(not(any(target_os = "macos", windows)))]
use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[cfg(any(target_os = "macos", windows))]
const KEYCHAIN_SERVICE: &str = "scope-vcs";
#[cfg(any(target_os = "macos", windows))]
static SET_CREDENTIAL_STORE: Once = Once::new();

pub fn cached_cli_session(
    client: &Client,
    api_url: &str,
) -> anyhow::Result<Option<AuthenticatedSession>> {
    let Some(token) = read_stored_session_token(api_url)? else {
        return Ok(None);
    };

    match validate_session_token(client, api_url, &token)? {
        Some(user) => Ok(Some(AuthenticatedSession { token, user })),
        None => {
            delete_stored_session_token(api_url)?;
            Ok(None)
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn read_stored_session_token(api_url: &str) -> anyhow::Result<Option<String>> {
    let path = session_file_path(api_url)?;
    match fs::read_to_string(&path) {
        Ok(token) if token.trim().is_empty() => Ok(None),
        Ok(token) => Ok(Some(token.trim().to_string())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| {
            session_file_error_context("read Scope CLI session from local session file")
        }),
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn store_session_token(api_url: &str, session_token: &str) -> anyhow::Result<()> {
    let path = session_file_path(api_url)?;
    let parent = path
        .parent()
        .context("Scope CLI session file path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| session_file_error_context("create Scope CLI session directory"))?;
    restrict_directory_to_owner(parent)
        .with_context(|| session_file_error_context("secure Scope CLI session directory"))?;

    let temp_path = path.with_file_name(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("scope-session"),
        std::process::id()
    ));
    write_session_file(&temp_path, session_token)
        .with_context(|| session_file_error_context("write Scope CLI session file"))?;
    fs::rename(&temp_path, &path)
        .with_context(|| session_file_error_context("replace Scope CLI session file"))?;
    restrict_file_to_owner(&path)
        .with_context(|| session_file_error_context("secure Scope CLI session file"))?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn delete_stored_session_token(api_url: &str) -> anyhow::Result<()> {
    let path = session_file_path(api_url)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| session_file_error_context("delete Scope CLI session file"))
        }
    }
}

#[cfg(any(target_os = "macos", windows))]
pub fn read_stored_session_token(api_url: &str) -> anyhow::Result<Option<String>> {
    let entry = session_keychain_entry(api_url)?;
    match entry.get_password() {
        Ok(token) if token.trim().is_empty() => Ok(None),
        Ok(token) => Ok(Some(token)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(error) => Err(error).context(keychain_error_context(
            "read Scope CLI session from OS keychain",
        )),
    }
}

#[cfg(any(target_os = "macos", windows))]
pub fn store_session_token(api_url: &str, session_token: &str) -> anyhow::Result<()> {
    let entry = session_keychain_entry(api_url)?;
    entry
        .set_password(session_token)
        .context(keychain_error_context(
            "store Scope CLI session in OS keychain",
        ))
}

#[cfg(any(target_os = "macos", windows))]
pub fn delete_stored_session_token(api_url: &str) -> anyhow::Result<()> {
    let entry = session_keychain_entry(api_url)?;
    match entry.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(error) => Err(error).context(keychain_error_context(
            "delete Scope CLI session from OS keychain",
        )),
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn session_file_path(api_url: &str) -> anyhow::Result<PathBuf> {
    let config_dir = scope_config_dir(
        env::var_os("XDG_CONFIG_HOME"),
        env::var_os("HOME"),
        env::var_os("USERPROFILE"),
    )
    .context(session_file_error_context(
        "locate Scope CLI session directory",
    ))?;
    Ok(config_dir
        .join("scope")
        .join("sessions")
        .join(session_storage_key(api_url)))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn scope_config_dir(
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
    userprofile: Option<OsString>,
) -> Option<PathBuf> {
    non_empty_path(xdg_config_home)
        .or_else(|| non_empty_path(home).map(|path| path.join(".config")))
        .or_else(|| non_empty_path(userprofile).map(|path| path.join(".config")))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn non_empty_path(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|path| !path.is_empty()).map(PathBuf::from)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn write_session_file(path: &Path, session_token: &str) -> anyhow::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(session_token.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn restrict_directory_to_owner(path: &Path) -> io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn restrict_file_to_owner(path: &Path) -> io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn session_file_error_context(action: &str) -> String {
    format!(
        "{action}\n\nScope stores CLI sessions in a local file on Linux with owner-only permissions. Make sure XDG_CONFIG_HOME or HOME points to a writable directory."
    )
}

#[cfg(any(target_os = "macos", windows))]
fn session_keychain_entry(api_url: &str) -> anyhow::Result<Entry> {
    install_native_credential_store();
    Entry::new(KEYCHAIN_SERVICE, &session_storage_key(api_url)).context(keychain_error_context(
        "open OS keychain entry for Scope CLI session",
    ))
}

#[cfg(any(target_os = "macos", windows))]
fn install_native_credential_store() {
    SET_CREDENTIAL_STORE.call_once(|| {
        #[cfg(target_os = "macos")]
        {
            if let Ok(store) = apple_native_keyring_store::keychain::Store::new() {
                keyring_core::set_default_store(store);
            }
        }
        #[cfg(windows)]
        {
            if let Ok(store) = windows_native_keyring_store::Store::new() {
                keyring_core::set_default_store(store);
            }
        }
    });
}

#[cfg(any(target_os = "macos", windows))]
fn keychain_error_context(action: &str) -> String {
    format!(
        "{action}\n\nScope stores CLI sessions in the OS keychain. Make sure the OS credential store is available and unlocked."
    )
}

fn session_storage_key(api_url: &str) -> String {
    let mut encoded = String::with_capacity(api_url.len() * 2);
    for byte in api_url.bytes() {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a string cannot fail");
    }
    format!("cli-session-{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_storage_key_is_scoped_to_api_url() {
        let production = session_storage_key("https://scope-api-production.up.railway.app");
        assert!(production.starts_with("cli-session-"));
        assert_ne!(production, session_storage_key("http://localhost:8080"));
    }
}
