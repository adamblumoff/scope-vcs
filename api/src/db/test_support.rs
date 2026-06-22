use super::{
    DbRuntime, MetadataStore, MetadataStoreInner, ensure_metadata_lock_row, run_db_on, schema,
};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) struct TestDatabaseTarget {
    database_url: String,
    schema_name: String,
}

impl TestDatabaseTarget {
    pub(crate) fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(database_url) = std::env::var("SCOPE_TEST_DATABASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(None);
        };
        validate_test_database_url(&database_url)?;
        Ok(Some(Self {
            database_url,
            schema_name: unique_test_schema_name(),
        }))
    }
}

pub(super) fn connect_postgres_test_store(
    target: &TestDatabaseTarget,
    reset_schema: bool,
) -> anyhow::Result<MetadataStore> {
    let runtime = DbRuntime::new()?;
    let database_url = target.database_url.clone();
    let schema_name = target.schema_name.clone();
    let db = run_db_on(&runtime, async move {
        let admin = Database::connect(&database_url).await?;
        admin
            .execute(Statement::from_string(
                admin.get_database_backend(),
                format!(
                    "CREATE SCHEMA IF NOT EXISTS {}",
                    quote_pg_ident(&schema_name)
                ),
            ))
            .await?;

        let mut options = ConnectOptions::new(database_url);
        options.max_connections(1).min_connections(1);
        let db = Database::connect(options).await?;
        db.execute(Statement::from_string(
            db.get_database_backend(),
            format!("SET search_path TO {}", quote_pg_ident(&schema_name)),
        ))
        .await?;
        if reset_schema {
            schema::reset_metadata_schema(&db).await?;
        }
        schema::migrate_metadata_schema(&db).await?;
        ensure_metadata_lock_row(&db).await?;
        Ok::<_, sea_orm::DbErr>(db)
    })?;

    Ok(MetadataStore {
        inner: Arc::new(MetadataStoreInner::Postgres {
            db: Arc::new(db),
            runtime,
        }),
    })
}

fn validate_test_database_url(database_url: &str) -> anyhow::Result<()> {
    let lower = database_url.trim().to_ascii_lowercase();
    if !(lower.starts_with("postgres://") || lower.starts_with("postgresql://")) {
        anyhow::bail!("SCOPE_TEST_DATABASE_URL must be a postgres:// or postgresql:// URL");
    }

    let after_scheme = lower
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
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
            ) && has_scope_test_marker(value)
        });
    let has_test_marker = has_scope_test_marker(database_name) || query_has_schema_marker;

    if !has_test_marker {
        anyhow::bail!(
            "SCOPE_TEST_DATABASE_URL must visibly target a Scope test database or schema; include scope_test, scope-test, scope_vcs_test, or scope-vcs-test in the database name or search_path/schema query"
        );
    }
    Ok(())
}

fn has_scope_test_marker(value: &str) -> bool {
    value.contains("scope_test")
        || value.contains("scope-test")
        || value.contains("scope_vcs_test")
        || value.contains("scope-vcs-test")
}

fn unique_test_schema_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after UNIX epoch")
        .as_nanos();
    format!("scope_test_{}_{}", std::process::id(), nanos)
}

fn quote_pg_ident(identifier: &str) -> String {
    assert!(
        identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
        "generated test schema identifiers only use postgres-safe characters"
    );
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_url_requires_scope_test_marker() {
        let error = validate_test_database_url("postgres://localhost/scope_staging").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("must visibly target a Scope test database or schema")
        );

        validate_test_database_url("postgres://localhost/scope_test").unwrap();
        validate_test_database_url("postgres://localhost/scope-vcs-test").unwrap();
        validate_test_database_url("postgres://localhost/postgres?search_path=scope_test_run")
            .unwrap();

        let error =
            validate_test_database_url("postgres://localhost/prod?application_name=scope_test")
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("must visibly target a Scope test database or schema")
        );
        let error =
            validate_test_database_url("postgres://localhost/prod?foo=scope_test").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("must visibly target a Scope test database or schema")
        );
    }

    #[test]
    fn test_database_url_must_be_postgres() {
        let error = validate_test_database_url("sqlite://scope_test").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("must be a postgres:// or postgresql:// URL")
        );
    }
}
