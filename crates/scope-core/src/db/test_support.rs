#[cfg(any(test, feature = "test-support"))]
use super::load_catalog;
use super::{
    MetadataStore, acquire_aggregate_lock,
    cleanup_queue::{save_pending_repo_storage_deletions, save_pending_source_blob_deletions},
    ensure_metadata_lock_row, entities,
    repository_rows::insert_repository,
    request_rows::{
        insert_credit_ledger_entry_row, insert_request_event_row, insert_request_row,
        save_credit_account_row,
    },
    schema,
};
use crate::{domain::store::AppCatalog, error::ApiError};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, TransactionTrait};
#[cfg(any(test, feature = "test-support"))]
use sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};
#[cfg(any(test, feature = "test-support"))]
use std::{
    future::Future,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug)]
pub struct TestDatabaseTarget {
    database_url: String,
    schema_name: String,
}

#[cfg(any(test, feature = "test-support"))]
pub(super) struct TestSchemaLease {
    database: Arc<sea_orm::DatabaseConnection>,
    database_url: String,
    schema_name: String,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for TestSchemaLease {
    fn drop(&mut self) {
        let database_url = self.database_url.clone();
        let schema_name = self.schema_name.clone();
        let database = Arc::clone(&self.database);
        test_runtime().spawn(async move {
            let _ = database.close_by_ref().await;
            let Ok(db) = Database::connect(database_url).await else {
                return;
            };
            let _ = db
                .execute(Statement::from_string(
                    db.get_database_backend(),
                    format!(
                        "DROP SCHEMA IF EXISTS {} CASCADE",
                        quote_pg_ident(&schema_name)
                    ),
                ))
                .await;
            let _ = db.close().await;
        });
    }
}

#[cfg(any(test, feature = "test-support"))]
impl TestDatabaseTarget {
    pub fn required() -> anyhow::Result<Self> {
        let database_url = std::env::var("SCOPE_TEST_DATABASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "postgres://scope:scope@127.0.0.1:5432/scope_test".to_string());
        validate_test_database_url(&database_url)?;
        Ok(Self {
            database_url,
            schema_name: unique_test_schema_name(),
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn connect_postgres_test_store(
    target: &TestDatabaseTarget,
    reset_schema: bool,
) -> anyhow::Result<MetadataStore> {
    let database_url = target.database_url.clone();
    let schema_name = target.schema_name.clone();
    let db = run_test_future(async move {
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
        options
            .max_connections(8)
            .min_connections(1)
            .set_schema_search_path(schema_name);
        let db = Database::connect(options).await?;
        if reset_schema {
            schema::reset_metadata_schema(&db).await?;
        }
        schema::migrate_metadata_schema(&db).await?;
        ensure_metadata_lock_row(&db).await?;
        Ok::<_, sea_orm::DbErr>(db)
    })?;

    let db = Arc::new(db);
    let test_schema = TestSchemaLease {
        database: Arc::clone(&db),
        database_url: target.database_url.clone(),
        schema_name: target.schema_name.clone(),
    };
    Ok(MetadataStore {
        db,
        postgres_database_url: Some(Arc::from(target.database_url.clone())),
        _test_schema: Some(Arc::new(test_schema)),
    })
}

impl MetadataStore {
    #[cfg(feature = "local-dev")]
    pub async fn replace_catalog_for_local_dev(&self, catalog: AppCatalog) -> Result<(), ApiError> {
        replace_catalog(self.db.as_ref(), catalog).await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn update<R>(
        &self,
        op: impl FnOnce(&mut AppCatalog) -> Result<R, ApiError>,
    ) -> Result<R, ApiError> {
        let db = Arc::clone(&self.db);
        let mut catalog = run_test_future(async move { load_catalog(db.as_ref()).await })?;
        let result = op(&mut catalog)?;
        self.replace_catalog_for_tests(catalog)?;
        Ok(result)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn seed_catalog_for_tests(&self, catalog: AppCatalog) -> Result<(), ApiError> {
        let db = Arc::clone(&self.db);
        run_test_future(async move { seed_catalog(db.as_ref(), catalog).await })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn replace_catalog_for_tests(&self, catalog: AppCatalog) -> Result<(), ApiError> {
        let db = Arc::clone(&self.db);
        run_test_future(async move { replace_catalog(db.as_ref(), catalog).await })
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn test_catalog(store: &MetadataStore) -> Result<TestCatalogGuard, ApiError> {
    let db = Arc::clone(&store.db);
    let catalog = run_test_future(async move { load_catalog(db.as_ref()).await })?;
    Ok(TestCatalogGuard::new(store.clone(), catalog))
}

async fn replace_catalog(
    db: &sea_orm::DatabaseConnection,
    catalog: AppCatalog,
) -> Result<(), ApiError> {
    let reset_events = entities::metadata_reset_event::Entity::find()
        .all(db)
        .await
        .map_err(ApiError::internal)?;
    schema::reset_metadata_schema(db)
        .await
        .map_err(ApiError::internal)?;
    schema::migrate_metadata_schema(db)
        .await
        .map_err(ApiError::internal)?;
    ensure_metadata_lock_row(db)
        .await
        .map_err(ApiError::internal)?;
    for event in reset_events {
        event
            .into_active_model()
            .insert(db)
            .await
            .map_err(ApiError::internal)?;
    }
    seed_catalog(db, catalog).await
}

#[cfg(any(test, feature = "test-support"))]
fn run_test_future<R: Send + 'static>(future: impl Future<Output = R> + Send + 'static) -> R {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    test_runtime().spawn(async move {
        let _ = sender.send(future.await);
    });
    receiver
        .recv()
        .expect("test database runtime should not stop")
}

#[cfg(any(test, feature = "test-support"))]
fn test_runtime() -> &'static tokio::runtime::Handle {
    static HANDLE: OnceLock<tokio::runtime::Handle> = OnceLock::new();
    HANDLE.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new()
                .expect("creating test database runtime should succeed");
            sender.send(runtime.handle().clone()).unwrap();
            runtime.block_on(std::future::pending::<()>());
        });
        receiver.recv().expect("test database runtime should start")
    })
}

async fn seed_catalog(
    conn: &sea_orm::DatabaseConnection,
    mut catalog: AppCatalog,
) -> Result<(), ApiError> {
    complete_test_users(&mut catalog);
    let tx = conn.begin().await.map_err(ApiError::internal)?;
    acquire_aggregate_lock(&tx, "test", "catalog").await?;
    for user in catalog.users.values() {
        entities::user::Model::from_domain(user)
            .into_active_model()
            .insert(&tx)
            .await
            .map_err(ApiError::internal)?;
    }
    for repo in catalog.repositories.values() {
        insert_repository(&tx, repo).await?;
    }
    for request in catalog.requests.values() {
        insert_request_row(&tx, request).await?;
    }
    for event in catalog.request_events.values() {
        insert_request_event_row(&tx, event).await?;
    }
    for account in catalog.user_credit_accounts.values() {
        save_credit_account_row(&tx, account).await?;
    }
    for entry in catalog.credit_ledger_entries.values() {
        insert_credit_ledger_entry_row(&tx, entry).await?;
    }
    save_pending_repo_storage_deletions(&tx, &catalog.pending_repo_storage_deletions).await?;
    save_pending_source_blob_deletions(&tx, &catalog.pending_source_blob_deletions).await?;
    tx.commit().await.map_err(ApiError::internal)
}

fn complete_test_users(catalog: &mut AppCatalog) {
    let identities = catalog.repositories.values().flat_map(|repo| {
        std::iter::once((
            repo.record.owner_user_id.clone(),
            repo.record.owner_handle.clone(),
        ))
        .chain(
            repo.members
                .iter()
                .map(|member| (member.user_id.clone(), member.user_id.clone())),
        )
    });
    for (id, handle) in identities.collect::<Vec<_>>() {
        catalog
            .users
            .entry(id.clone())
            .or_insert_with(|| crate::domain::store::UserAccount {
                id: id.clone(),
                handle,
                email: format!("{id}@scope.test"),
                email_verified: true,
            });
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct TestCatalogGuard {
    store: MetadataStore,
    catalog: AppCatalog,
    dirty: bool,
}

#[cfg(any(test, feature = "test-support"))]
impl TestCatalogGuard {
    pub(super) fn new(store: MetadataStore, catalog: AppCatalog) -> Self {
        Self {
            store,
            catalog,
            dirty: false,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl std::ops::Deref for TestCatalogGuard {
    type Target = AppCatalog;

    fn deref(&self) -> &Self::Target {
        &self.catalog
    }
}

#[cfg(any(test, feature = "test-support"))]
impl std::ops::DerefMut for TestCatalogGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.dirty = true;
        &mut self.catalog
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for TestCatalogGuard {
    fn drop(&mut self) {
        if self.dirty {
            self.store
                .replace_catalog_for_tests(self.catalog.clone())
                .expect("persisting test catalog mutation should succeed");
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
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

#[cfg(any(test, feature = "test-support"))]
fn has_scope_test_marker(value: &str) -> bool {
    value.contains("scope_test")
        || value.contains("scope-test")
        || value.contains("scope_vcs_test")
        || value.contains("scope-vcs-test")
}

#[cfg(any(test, feature = "test-support"))]
fn unique_test_schema_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after UNIX epoch")
        .as_nanos();
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("scope_test_{}_{}_{}", std::process::id(), nanos, sequence)
}

#[cfg(any(test, feature = "test-support"))]
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
