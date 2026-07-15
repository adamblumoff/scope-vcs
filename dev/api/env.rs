use crate::config::{
    CLERK_AUTHORIZED_PARTIES_ENV, CLERK_ISSUER_ENV, DATABASE_URL_ENV, LOCAL_API_ORIGIN,
    LOCAL_APP_ORIGIN, SCOPE_API_PUBLIC_URL_ENV, SCOPE_APP_ORIGIN_ENV, SCOPE_OBJECT_STORE_ENV,
    non_empty_env,
};

pub(super) const SCOPE_ENV_ENV: &str = "SCOPE_ENV";
pub(super) const LOCAL_SCOPE_ENV: &str = "local";
pub(super) const SCOPE_DEV_USER_EMAIL_ENV: &str = "SCOPE_DEV_USER_EMAIL";
pub(super) const SCOPE_DEV_USER_HANDLE_ENV: &str = "SCOPE_DEV_USER_HANDLE";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DevSeedUser {
    pub(super) email: String,
    pub(super) handle: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LocalDevSettings {
    pub(super) seed_user: DevSeedUser,
}

pub fn is_local_dev_env() -> bool {
    non_empty_env(SCOPE_ENV_ENV).as_deref() == Some(LOCAL_SCOPE_ENV)
}

pub(super) fn validate_local_dev_environment() -> anyhow::Result<LocalDevSettings> {
    let snapshot = DevEnvSnapshot::from_env();
    validate_snapshot(&snapshot)
}

#[derive(Default)]
struct DevEnvSnapshot {
    scope_env: Option<String>,
    database_url: Option<String>,
    app_origin: Option<String>,
    api_public_url: Option<String>,
    clerk_issuer: Option<String>,
    clerk_authorized_parties: Option<String>,
    object_store: Option<String>,
    railway_environment_name: Option<String>,
    railway_project_id: Option<String>,
    dev_user_email: Option<String>,
    dev_user_handle: Option<String>,
}

impl DevEnvSnapshot {
    fn from_env() -> Self {
        Self {
            scope_env: non_empty_env(SCOPE_ENV_ENV),
            database_url: non_empty_env(DATABASE_URL_ENV),
            app_origin: non_empty_env(SCOPE_APP_ORIGIN_ENV),
            api_public_url: non_empty_env(SCOPE_API_PUBLIC_URL_ENV),
            clerk_issuer: non_empty_env(CLERK_ISSUER_ENV),
            clerk_authorized_parties: non_empty_env(CLERK_AUTHORIZED_PARTIES_ENV),
            object_store: non_empty_env(SCOPE_OBJECT_STORE_ENV),
            railway_environment_name: non_empty_env("RAILWAY_ENVIRONMENT_NAME"),
            railway_project_id: non_empty_env("RAILWAY_PROJECT_ID"),
            dev_user_email: non_empty_env(SCOPE_DEV_USER_EMAIL_ENV),
            dev_user_handle: non_empty_env(SCOPE_DEV_USER_HANDLE_ENV),
        }
    }
}

fn validate_snapshot(snapshot: &DevEnvSnapshot) -> anyhow::Result<LocalDevSettings> {
    require_exact(
        SCOPE_ENV_ENV,
        snapshot.scope_env.as_deref(),
        LOCAL_SCOPE_ENV,
    )?;
    require_exact(
        SCOPE_APP_ORIGIN_ENV,
        snapshot.app_origin.as_deref(),
        LOCAL_APP_ORIGIN,
    )?;
    require_exact(
        SCOPE_API_PUBLIC_URL_ENV,
        snapshot.api_public_url.as_deref(),
        LOCAL_API_ORIGIN,
    )?;

    if snapshot.railway_environment_name.is_some() || snapshot.railway_project_id.is_some() {
        anyhow::bail!("{SCOPE_ENV_ENV}=local cannot run with Railway environment variables");
    }

    let object_store = snapshot.object_store.as_deref().unwrap_or("filesystem");
    if object_store != "filesystem" {
        anyhow::bail!("{SCOPE_ENV_ENV}=local requires {SCOPE_OBJECT_STORE_ENV}=filesystem");
    }

    let database_url = snapshot
        .database_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("{DATABASE_URL_ENV} is required in local dev"))?;
    validate_local_database_url(database_url)?;

    let clerk_issuer = snapshot
        .clerk_issuer
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("{CLERK_ISSUER_ENV} is required in local dev"))?;
    validate_local_clerk_issuer(clerk_issuer)?;

    if let Some(authorized_parties) = snapshot.clerk_authorized_parties.as_deref()
        && !configured_list_contains(authorized_parties, LOCAL_APP_ORIGIN)
    {
        anyhow::bail!("{CLERK_AUTHORIZED_PARTIES_ENV} must include {LOCAL_APP_ORIGIN}");
    }

    Ok(LocalDevSettings {
        seed_user: dev_seed_user(snapshot)?,
    })
}

fn require_exact(name: &str, actual: Option<&str>, expected: &str) -> anyhow::Result<()> {
    match actual {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => anyhow::bail!("{name} must be {expected} in local dev, got {actual}"),
        None => anyhow::bail!("{name} must be {expected} in local dev"),
    }
}

fn validate_local_clerk_issuer(issuer: &str) -> anyhow::Result<()> {
    let issuer = issuer.trim_end_matches('/');
    if issuer.contains("clerk.scopevcs.com") || issuer.contains("scopevcs.com") {
        anyhow::bail!("{CLERK_ISSUER_ENV} must not point at production Clerk in local dev");
    }
    if !issuer.ends_with(".clerk.accounts.dev") {
        anyhow::bail!("{CLERK_ISSUER_ENV} must point at a Clerk development issuer");
    }
    Ok(())
}

fn validate_local_database_url(database_url: &str) -> anyhow::Result<()> {
    reject_production_database_url(database_url)?;

    let lower = database_url.trim().to_ascii_lowercase();
    if !(lower.starts_with("postgres://") || lower.starts_with("postgresql://")) {
        anyhow::bail!("{DATABASE_URL_ENV} must be a postgres:// or postgresql:// URL");
    }

    let after_scheme = lower
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    let authority = after_scheme.split('/').next().unwrap_or_default();
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host_port
        .trim_start_matches('[')
        .split(']')
        .next()
        .unwrap_or(host_port)
        .split(':')
        .next()
        .unwrap_or_default();
    if !matches!(host, "localhost" | "127.0.0.1" | "::1") {
        anyhow::bail!("{DATABASE_URL_ENV} must target localhost in local dev");
    }

    let database_and_query = after_scheme
        .split_once('/')
        .map(|(_, path)| path)
        .unwrap_or_default();
    let database_name = database_and_query
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    let query = database_and_query
        .split_once('?')
        .map(|(_, query)| query.split('#').next().unwrap_or_default())
        .unwrap_or_default();
    let query_has_schema_marker = query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .any(|(key, value)| {
            matches!(
                key,
                "search_path" | "schema" | "current_schema" | "currentschema"
            ) && has_local_database_marker(value)
        });

    if !has_local_database_marker(database_name) && !query_has_schema_marker {
        anyhow::bail!("{DATABASE_URL_ENV} must visibly target a Scope local/dev database");
    }

    Ok(())
}

fn reject_production_database_url(database_url: &str) -> anyhow::Result<()> {
    let lower = database_url.trim().to_ascii_lowercase();
    if lower.contains("railway") || lower.contains("scope-postgres") || lower.contains("production")
    {
        anyhow::bail!("{DATABASE_URL_ENV} must not point at production data in local dev");
    }
    Ok(())
}

fn has_local_database_marker(value: &str) -> bool {
    value.contains("scope_dev")
        || value.contains("scope-dev")
        || value.contains("scope_local")
        || value.contains("scope-local")
        || value.contains("scope_vcs_dev")
        || value.contains("scope-vcs-dev")
        || value.contains("scope_vcs_local")
        || value.contains("scope-vcs-local")
        || value.contains("scope_test")
        || value.contains("scope-test")
}

fn configured_list_contains(values: &str, expected: &str) -> bool {
    values
        .split(',')
        .map(str::trim)
        .any(|value| value == expected)
}

fn dev_seed_user(snapshot: &DevEnvSnapshot) -> anyhow::Result<DevSeedUser> {
    let email = snapshot
        .dev_user_email
        .as_deref()
        .map(normalize_email)
        .filter(|email| email.contains('@'))
        .ok_or_else(|| {
            anyhow::anyhow!("{SCOPE_DEV_USER_EMAIL_ENV} must be set to your Clerk dev email")
        })?;
    let handle = match snapshot.dev_user_handle.as_deref() {
        Some(raw) => normalize_handle(raw).ok_or_else(|| {
            anyhow::anyhow!("{SCOPE_DEV_USER_HANDLE_ENV} must contain letters or numbers")
        })?,
        None => email
            .split('@')
            .next()
            .and_then(normalize_handle)
            .unwrap_or_else(|| "dev-user".to_string()),
    };

    Ok(DevSeedUser { email, handle })
}

fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn normalize_handle(value: &str) -> Option<String> {
    let mut handle = String::new();
    let mut last_was_separator = false;
    for byte in value.trim().bytes() {
        let next = if byte.is_ascii_alphanumeric() {
            last_was_separator = false;
            Some(byte.to_ascii_lowercase() as char)
        } else if matches!(byte, b'-' | b'_') && !last_was_separator {
            last_was_separator = true;
            Some('-')
        } else {
            None
        };

        if let Some(next) = next {
            handle.push(next);
        }
    }

    let handle = handle.trim_matches('-').to_string();
    if handle.is_empty() || handle.len() > 40 {
        None
    } else {
        Some(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_runtime_accepts_safe_postgres_config() {
        let settings = validate_snapshot(&local_snapshot()).unwrap();
        assert_eq!(settings.seed_user.email, "dev@example.com");
        assert_eq!(settings.seed_user.handle, "dev");
    }

    #[test]
    fn local_runtime_requires_database() {
        let snapshot = DevEnvSnapshot {
            database_url: None,
            ..local_snapshot()
        };

        let error = validate_snapshot(&snapshot).unwrap_err();
        assert!(error.to_string().contains(DATABASE_URL_ENV));
    }

    #[test]
    fn local_runtime_rejects_railway_markers() {
        let snapshot = DevEnvSnapshot {
            railway_environment_name: Some("production".to_string()),
            ..local_snapshot()
        };

        let error = validate_snapshot(&snapshot).unwrap_err();

        assert!(error.to_string().contains("Railway environment variables"));
    }

    #[test]
    fn local_runtime_rejects_production_database() {
        let snapshot = DevEnvSnapshot {
            database_url: Some(
                "postgres://user:pass@scope-postgres.railway.internal:5432/railway".to_string(),
            ),
            ..local_snapshot()
        };

        let error = validate_snapshot(&snapshot).unwrap_err();

        assert!(error.to_string().contains("production data"));
    }

    #[test]
    fn local_postgres_requires_localhost_and_scope_marker() {
        validate_local_database_url("postgres://scope:scope@127.0.0.1:5432/scope_dev").unwrap();
        validate_local_database_url(
            "postgres://scope:scope@localhost/postgres?search_path=scope_test",
        )
        .unwrap();

        let error = validate_local_database_url("postgres://scope:scope@db.example.com/scope_dev")
            .unwrap_err();
        assert!(error.to_string().contains("localhost"));

        let error =
            validate_local_database_url("postgres://scope:scope@localhost/postgres").unwrap_err();
        assert!(error.to_string().contains("local/dev database"));
    }

    #[test]
    fn local_runtime_rejects_production_clerk_issuer() {
        let snapshot = DevEnvSnapshot {
            clerk_issuer: Some("https://clerk.scopevcs.com".to_string()),
            ..local_snapshot()
        };

        let error = validate_snapshot(&snapshot).unwrap_err();

        assert!(error.to_string().contains("production Clerk"));
    }

    #[test]
    fn local_runtime_requires_seed_user_email() {
        let snapshot = DevEnvSnapshot {
            dev_user_email: None,
            ..local_snapshot()
        };

        let error = validate_snapshot(&snapshot).unwrap_err();

        assert!(error.to_string().contains(SCOPE_DEV_USER_EMAIL_ENV));
    }

    #[test]
    fn local_runtime_derives_seed_handle_from_email() {
        let snapshot = DevEnvSnapshot {
            dev_user_email: Some("Dev.User+local@Example.COM".to_string()),
            dev_user_handle: None,
            ..local_snapshot()
        };

        let settings = validate_snapshot(&snapshot).unwrap();

        assert_eq!(settings.seed_user.email, "dev.user+local@example.com");
        assert_eq!(settings.seed_user.handle, "devuserlocal");
    }

    fn local_snapshot() -> DevEnvSnapshot {
        DevEnvSnapshot {
            scope_env: Some(LOCAL_SCOPE_ENV.to_string()),
            app_origin: Some(LOCAL_APP_ORIGIN.to_string()),
            api_public_url: Some(LOCAL_API_ORIGIN.to_string()),
            clerk_issuer: Some("https://scope-dev.clerk.accounts.dev".to_string()),
            clerk_authorized_parties: Some(LOCAL_APP_ORIGIN.to_string()),
            object_store: Some("filesystem".to_string()),
            database_url: Some("postgres://scope:scope@127.0.0.1:5432/scope_dev".to_string()),
            dev_user_email: Some("dev@example.com".to_string()),
            dev_user_handle: Some("dev".to_string()),
            ..DevEnvSnapshot::default()
        }
    }
}
