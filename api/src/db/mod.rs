mod entities;
mod schema;

use crate::domain::store::{AppCatalog, RepoMembership};
use crate::error::ApiError;
#[cfg(test)]
use sea_orm::ConnectOptions;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, Database, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Set, Statement, TransactionTrait,
    TryInsertResult,
    sea_query::{LockType, OnConflict},
};
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

const METADATA_LOCK_KEY: &str = "catalog";

#[derive(Clone)]
pub(crate) struct MetadataStore {
    inner: Arc<MetadataStoreInner>,
}

enum MetadataStoreInner {
    Postgres {
        db: Arc<DatabaseConnection>,
        runtime: DbRuntime,
    },
    #[cfg(test)]
    Memory(Arc<MemoryMetadataStore>),
}

#[cfg(test)]
struct MemoryMetadataStore {
    catalog: std::sync::Mutex<AppCatalog>,
    fail_next_persist: AtomicBool,
}

struct DbRuntime(Option<tokio::runtime::Runtime>);

impl DbRuntime {
    fn new() -> anyhow::Result<Self> {
        Ok(Self(Some(tokio::runtime::Runtime::new()?)))
    }
}

impl std::ops::Deref for DbRuntime {
    type Target = tokio::runtime::Runtime;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("database runtime has already shut down")
    }
}

impl Drop for DbRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            // This store is process-lifetime state. Dropping a Tokio runtime from
            // inside the server runtime panics, so let the OS reclaim it on exit.
            std::mem::forget(runtime);
        }
    }
}

impl MetadataStore {
    pub(crate) fn connect_from_env() -> anyhow::Result<Self> {
        let database_url = std::env::var(crate::config::DATABASE_URL_ENV)
            .map_err(|_| anyhow::anyhow!("DATABASE_URL is required for Scope metadata storage"))?;
        connect_postgres_store(database_url)
    }

    #[cfg(test)]
    pub(crate) fn memory(catalog: AppCatalog) -> Self {
        Self {
            inner: Arc::new(MetadataStoreInner::Memory(Arc::new(MemoryMetadataStore {
                catalog: std::sync::Mutex::new(catalog),
                fail_next_persist: AtomicBool::new(false),
            }))),
        }
    }

    #[cfg(test)]
    pub(crate) fn connect_for_tests(target: &TestDatabaseTarget) -> anyhow::Result<Self> {
        connect_postgres_test_store(target, false)
    }

    #[cfg(test)]
    pub(crate) fn connect_fresh_for_tests(target: &TestDatabaseTarget) -> anyhow::Result<Self> {
        connect_postgres_test_store(target, true)
    }

    pub(crate) fn read<R>(
        &self,
        op: impl FnOnce(&AppCatalog) -> Result<R, ApiError>,
    ) -> Result<R, ApiError>
    where
        R: Send + 'static,
    {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let catalog = load_catalog(runtime, db)?;
                op(&catalog)
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(catalog) => {
                let catalog = catalog
                    .catalog
                    .lock()
                    .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?;
                op(&catalog)
            }
        }
    }

    pub(crate) fn update<R, F>(&self, op: F) -> Result<R, ApiError>
    where
        R: Send + 'static,
        F: FnOnce(&mut AppCatalog) -> Result<R, ApiError> + Send + 'static,
    {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => update_catalog(runtime, db, op),
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut catalog = memory
                    .catalog
                    .lock()
                    .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?;
                let mut draft = catalog.clone();
                let result = op(&mut draft)?;
                if memory.fail_next_persist.swap(false, Ordering::SeqCst) {
                    return Err(ApiError::internal_message("test metadata persist failure"));
                }
                *catalog = draft;
                Ok(result)
            }
        }
    }

    pub(crate) fn readiness_check(&self) -> Result<(), ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    db.query_one(Statement::from_string(
                        db.get_database_backend(),
                        "SELECT 1".to_string(),
                    ))
                    .await
                    .map_err(ApiError::internal)?;
                    Ok(())
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(catalog) => {
                drop(
                    catalog
                        .catalog
                        .lock()
                        .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?,
                );
                Ok(())
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn test_catalog(&self) -> Result<std::sync::MutexGuard<'_, AppCatalog>, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Memory(catalog) => catalog
                .catalog
                .lock()
                .map_err(|_| ApiError::internal_message("catalog lock is poisoned")),
            MetadataStoreInner::Postgres { .. } => Err(ApiError::internal_message(
                "test catalog access is only available for memory metadata stores",
            )),
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_next_persist_for_tests(&self) -> Result<(), ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Memory(catalog) => {
                catalog.fail_next_persist.store(true, Ordering::SeqCst);
                Ok(())
            }
            MetadataStoreInner::Postgres { .. } => Err(ApiError::internal_message(
                "test persist failure is only available for memory metadata stores",
            )),
        }
    }
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct TestDatabaseTarget {
    database_url: String,
    schema_name: String,
}

#[cfg(test)]
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

fn connect_postgres_store(database_url: String) -> anyhow::Result<MetadataStore> {
    let runtime = DbRuntime::new()?;
    let db = run_db_on(&runtime, async move {
        let db = Database::connect(&database_url).await?;
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

#[cfg(test)]
fn connect_postgres_test_store(
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

#[cfg(test)]
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

#[cfg(test)]
fn has_scope_test_marker(value: &str) -> bool {
    value.contains("scope_test")
        || value.contains("scope-test")
        || value.contains("scope_vcs_test")
        || value.contains("scope-vcs-test")
}

#[cfg(test)]
fn unique_test_schema_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after UNIX epoch")
        .as_nanos();
    format!("scope_test_{}_{}", std::process::id(), nanos)
}

#[cfg(test)]
fn quote_pg_ident(identifier: &str) -> String {
    assert!(
        identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
        "generated test schema identifiers only use postgres-safe characters"
    );
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn load_catalog(
    runtime: &tokio::runtime::Runtime,
    db: &Arc<DatabaseConnection>,
) -> Result<AppCatalog, ApiError> {
    let db = Arc::clone(db);
    run_api_db_on(runtime, async move {
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_metadata_read_lock(&tx).await?;
        let catalog = load_catalog_async(&tx).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(catalog)
    })
}

fn update_catalog<R, F>(
    runtime: &tokio::runtime::Runtime,
    db: &Arc<DatabaseConnection>,
    op: F,
) -> Result<R, ApiError>
where
    R: Send + 'static,
    F: FnOnce(&mut AppCatalog) -> Result<R, ApiError> + Send + 'static,
{
    let db = Arc::clone(db);
    run_api_db_on(runtime, async move {
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_metadata_write_lock(&tx).await?;
        let mut catalog = load_catalog_async(&tx).await?;
        let result = op(&mut catalog)?;
        save_catalog_async(&tx, &catalog).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(result)
    })
}

async fn load_catalog_async<C>(conn: &C) -> Result<AppCatalog, ApiError>
where
    C: ConnectionTrait,
{
    let users = entities::user::Entity::find()
        .order_by_asc(entities::user::Column::Id)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    let repositories = entities::repository::Entity::find()
        .order_by_asc(entities::repository::Column::Id)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    let memberships = entities::membership::Entity::find()
        .order_by_asc(entities::membership::Column::RepoId)
        .order_by_asc(entities::membership::Column::UserId)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    let metadata_lock = entities::metadata_lock::Entity::find_by_id(METADATA_LOCK_KEY.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal_message("metadata lock row is missing"))?;

    let users = users
        .into_iter()
        .map(|user| Ok((user.id.clone(), user.try_into_domain()?)))
        .collect::<Result<_, ApiError>>()?;
    let memberships_by_repo = memberships.into_iter().try_fold(
        std::collections::BTreeMap::<String, Vec<RepoMembership>>::new(),
        |mut by_repo, membership| {
            let repo_id = membership.repo_id.clone();
            by_repo
                .entry(repo_id)
                .or_default()
                .push(membership.try_into_domain()?);
            Ok::<_, ApiError>(by_repo)
        },
    )?;
    let repositories = repositories
        .into_iter()
        .map(|repo| {
            let memberships = memberships_by_repo
                .get(&repo.id)
                .cloned()
                .unwrap_or_default();
            let repo = repo.try_into_domain(memberships)?;
            Ok((repo.record.id.clone(), repo))
        })
        .collect::<Result<_, ApiError>>()?;

    Ok(AppCatalog {
        users,
        repositories,
        pending_repo_storage_deletions: decode_json(metadata_lock.pending_repo_storage_deletions)?,
        pending_source_blob_deletions: decode_json(metadata_lock.pending_source_blob_deletions)?,
    })
}

async fn save_catalog_async<C>(conn: &C, catalog: &AppCatalog) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let users = catalog
        .users
        .values()
        .map(entities::user::Model::from_domain)
        .collect::<Vec<_>>();
    let repositories = catalog
        .repositories
        .values()
        .map(entities::repository::Model::from_domain)
        .collect::<Result<Vec<_>, ApiError>>()?;
    let memberships = catalog
        .repositories
        .values()
        .flat_map(|repo| repo.memberships.iter())
        .map(entities::membership::Model::from_domain)
        .collect::<Vec<_>>();

    entities::membership::Entity::delete_many()
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    entities::repository::Entity::delete_many()
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    entities::user::Entity::delete_many()
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;

    for user in users {
        user.into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }
    for repository in repositories {
        repository
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }
    for membership in memberships {
        membership
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }
    entities::metadata_lock::Entity::update_many()
        .filter(entities::metadata_lock::Column::Key.eq(METADATA_LOCK_KEY))
        .col_expr(
            entities::metadata_lock::Column::PendingRepoStorageDeletions,
            sea_orm::sea_query::Expr::value(encode_json(&catalog.pending_repo_storage_deletions)?),
        )
        .col_expr(
            entities::metadata_lock::Column::PendingSourceBlobDeletions,
            sea_orm::sea_query::Expr::value(encode_json(&catalog.pending_source_blob_deletions)?),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;

    Ok(())
}

async fn ensure_metadata_lock_row(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    match entities::metadata_lock::Entity::insert(entities::metadata_lock::ActiveModel {
        key: Set(METADATA_LOCK_KEY.to_string()),
        pending_repo_storage_deletions: Set(serde_json::Value::Array(Vec::new())),
        pending_source_blob_deletions: Set(serde_json::Value::Array(Vec::new())),
    })
    .on_conflict(
        OnConflict::column(entities::metadata_lock::Column::Key)
            .do_nothing()
            .to_owned(),
    )
    .do_nothing()
    .exec(db)
    .await?
    {
        TryInsertResult::Empty | TryInsertResult::Conflicted | TryInsertResult::Inserted(_) => {}
    }
    Ok(())
}

async fn acquire_metadata_read_lock<C>(conn: &C) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    acquire_metadata_lock(conn, LockType::Share).await
}

async fn acquire_metadata_write_lock<C>(conn: &C) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    acquire_metadata_lock(conn, LockType::Update).await
}

async fn acquire_metadata_lock<C>(conn: &C, lock: LockType) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let row = entities::metadata_lock::Entity::find_by_id(METADATA_LOCK_KEY.to_string())
        .lock(lock)
        .one(conn)
        .await
        .map_err(ApiError::internal)?;
    if row.is_none() {
        return Err(ApiError::internal_message("metadata lock row is missing"));
    }
    Ok(())
}

fn run_api_db_on<R>(
    runtime: &tokio::runtime::Runtime,
    future: impl std::future::Future<Output = Result<R, ApiError>> + Send + 'static,
) -> Result<R, ApiError>
where
    R: Send + 'static,
{
    run_on_runtime(runtime, future)
        .map_err(|error| ApiError::internal_message(error.to_string()))?
}

fn run_db_on<R>(
    runtime: &tokio::runtime::Runtime,
    future: impl std::future::Future<Output = Result<R, sea_orm::DbErr>> + Send + 'static,
) -> Result<R, sea_orm::DbErr>
where
    R: Send + 'static,
{
    run_on_runtime(runtime, future).map_err(|error| sea_orm::DbErr::Custom(error.to_string()))?
}

fn run_on_runtime<R, E>(
    runtime: &tokio::runtime::Runtime,
    future: impl std::future::Future<Output = Result<R, E>> + Send + 'static,
) -> Result<Result<R, E>, anyhow::Error>
where
    R: Send + 'static,
    E: Send + 'static,
{
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    runtime.spawn(async move {
        let _ = sender.send(future.await);
    });
    receiver
        .recv()
        .map_err(|_| anyhow::anyhow!("database runtime task was cancelled"))
}

fn encode_json<T: Serialize>(value: &T) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(value).map_err(ApiError::internal)
}

fn decode_json<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, ApiError> {
    serde_json::from_value(value).map_err(ApiError::internal)
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
