use crate::api::{AuthenticatedSession, validate_session_token};
use anyhow::{Context, bail};
use keyring::Entry;
use reqwest::blocking::Client;

const KEYCHAIN_SERVICE: &str = "scope-vcs";

pub fn ensure_cli_session(client: &Client, api_url: &str) -> anyhow::Result<AuthenticatedSession> {
    if let Some(session) = cached_cli_session(client, api_url)? {
        return Ok(session);
    }

    bail!("not signed in; run scope login before scope init")
}

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

pub fn read_stored_session_token(api_url: &str) -> anyhow::Result<Option<String>> {
    let entry = session_keychain_entry(api_url)?;
    match entry.get_password() {
        Ok(token) if token.trim().is_empty() => Ok(None),
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error).context(keychain_error_context(
            "read Scope CLI session from OS keychain",
        )),
    }
}

pub fn store_session_token(api_url: &str, session_token: &str) -> anyhow::Result<()> {
    let entry = session_keychain_entry(api_url)?;
    entry
        .set_password(session_token)
        .context(keychain_error_context(
            "store Scope CLI session in OS keychain",
        ))
}

pub fn delete_stored_session_token(api_url: &str) -> anyhow::Result<()> {
    let entry = session_keychain_entry(api_url)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error).context(keychain_error_context(
            "delete Scope CLI session from OS keychain",
        )),
    }
}

fn session_keychain_entry(api_url: &str) -> anyhow::Result<Entry> {
    Entry::new(KEYCHAIN_SERVICE, &session_keychain_username(api_url)).context(
        keychain_error_context("open OS keychain entry for Scope CLI session"),
    )
}

fn keychain_error_context(action: &str) -> String {
    format!("{action}\n\n{}", keychain_setup_help())
}

fn keychain_setup_help() -> &'static str {
    if cfg!(all(
        unix,
        not(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))
    )) {
        "Scope stores CLI sessions in the OS keychain. On Linux this requires a running, unlocked Secret Service provider.\n\nUbuntu/WSL setup:\n  sudo apt update\n  sudo apt install -y gnome-keyring dbus-x11 libsecret-tools\n  dbus-run-session -- bash\n\nThen, inside that fresh shell:\n  read -rsp \"Keyring password: \" KEYRING_PASSWORD; echo\n  printf %s \"$KEYRING_PASSWORD\" | gnome-keyring-daemon --unlock --components=secrets\n  unset KEYRING_PASSWORD\n  printf ok | secret-tool store --label=\"Scope keyring test\" scope test\n  secret-tool lookup scope test\n  secret-tool clear scope test\n  scope login\n\nIf you add the DBus/keyring startup commands to your shell profile, restart the shell or run `source ~/.bashrc` before running scope login. The important part is that scope must run from a shell that has the DBus/keyring environment variables.\n\nIf the test store still fails in WSL, back up and recreate the Linux keyring with:\n  cp -a ~/.local/share/keyrings ~/.local/share/keyrings.backup.$(date +%s) 2>/dev/null || true\n  rm -rf ~/.local/share/keyrings\n\nThen rerun the unlock and secret-tool test commands above from the same dbus-run-session shell."
    } else {
        "Scope stores CLI sessions in the OS keychain. Make sure the OS credential store is available and unlocked."
    }
}

fn session_keychain_username(api_url: &str) -> String {
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
    fn keychain_username_is_scoped_to_api_url() {
        assert_eq!(
            session_keychain_username("https://scope-api-production.up.railway.app"),
            "cli-session-68747470733a2f2f73636f70652d6170692d70726f64756374696f6e2e75702e7261696c7761792e617070"
        );
        assert_ne!(
            session_keychain_username("https://scope-api-production.up.railway.app"),
            session_keychain_username("http://localhost:8080")
        );
    }

    #[test]
    fn keychain_error_help_explains_linux_secret_service_setup() {
        let help = keychain_error_context("open OS keychain entry");
        assert!(help.contains("OS keychain"));
        if cfg!(all(
            unix,
            not(any(
                target_os = "macos",
                target_os = "ios",
                target_os = "android"
            ))
        )) {
            assert!(help.contains("Secret Service"));
            assert!(help.contains("gnome-keyring"));
            assert!(help.contains("dbus-run-session"));
            assert!(help.contains("restart the shell"));
            assert!(help.contains("source ~/.bashrc"));
        }
    }
}
