use super::{
    MetadataStore, acquire_aggregate_lock,
    cleanup_queue::{save_pending_repo_storage_deletions, save_pending_source_blob_deletions},
    ensure_metadata_lock_row, entities,
    repository_rows::insert_repository,
    request_change_block_rows::insert_change_block,
    request_discussion_rows::{insert_discussion, insert_reply, save_read_state},
    request_rows::{
        insert_credit_ledger_entry_row, insert_request_event_row, insert_request_row,
        save_credit_account_row,
    },
    schema,
};
#[cfg(any(test, feature = "test-support"))]
use super::{
    cleanup_queue::{
        load_pending_repo_storage_deletions, load_pending_source_blob_deletions,
        queue_pending_repo_storage_cleanup_row,
    },
    repository_from_model,
    repository_rows::save_repository_delta,
    request_rows::{request_by_id, save_request_row},
};
#[cfg(any(test, feature = "test-support"))]
use crate::domain::{
    requests::{CreditLedgerEntry, Request, RequestEvent, UserCreditAccount},
    store::{RepoStorageCleanup, SourceBlob, StoredRepository},
};
use crate::{
    domain::store::{AppCatalog, UserAccount},
    error::ApiError,
};
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
pub fn connect_postgres_test_store(target: &TestDatabaseTarget) -> anyhow::Result<MetadataStore> {
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
        schema::reset_metadata_schema(&db).await?;
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
    pub fn seed_catalog_for_tests(&self, catalog: AppCatalog) -> Result<(), ApiError> {
        let db = Arc::clone(&self.db);
        run_test_future(async move { seed_catalog(db.as_ref(), catalog).await })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl MetadataStore {
    pub async fn insert_user_for_tests(&self, user: UserAccount) -> Result<(), ApiError> {
        entities::user::Model::from_domain(&user)
            .into_active_model()
            .insert(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?;
        Ok(())
    }

    pub async fn queue_repo_storage_cleanup_for_tests(
        &self,
        cleanup: RepoStorageCleanup,
    ) -> Result<(), ApiError> {
        queue_pending_repo_storage_cleanup_row(self.db.as_ref(), cleanup).await
    }

    pub async fn pending_repo_storage_cleanups_for_tests(
        &self,
    ) -> Result<Vec<RepoStorageCleanup>, ApiError> {
        load_pending_repo_storage_deletions(self.db.as_ref()).await
    }

    pub async fn pending_source_blob_cleanups_for_tests(
        &self,
    ) -> Result<Vec<SourceBlob>, ApiError> {
        load_pending_source_blob_deletions(self.db.as_ref()).await
    }

    pub async fn replace_repository_for_tests(
        &self,
        repo: StoredRepository,
    ) -> Result<(), ApiError> {
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        ensure_repository_users_for_tests(&tx, &repo).await?;
        acquire_aggregate_lock(&tx, "repository", &repo.record.id).await?;
        match entities::repository::Entity::find_by_id(repo.record.id.clone())
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
        {
            Some(row) => {
                let before = repository_from_model(&tx, row).await?;
                save_repository_delta(&tx, &before, &repo).await?;
            }
            None => insert_repository(&tx, &repo).await?,
        }
        tx.commit().await.map_err(ApiError::internal)
    }

    pub async fn mutate_repository_for_tests(
        &self,
        repo_id: &str,
        op: impl FnOnce(&mut crate::domain::store::StoredRepository),
    ) -> Result<(), ApiError> {
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "repository", repo_id).await?;
        let row = entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found("test repository not found"))?;
        let mut repo = repository_from_model(&tx, row).await?;
        let before = repo.clone();
        op(&mut repo);
        save_repository_delta(&tx, &before, &repo).await?;
        tx.commit().await.map_err(ApiError::internal)
    }

    pub async fn insert_request_for_tests(&self, request: Request) -> Result<(), ApiError> {
        insert_request_row(self.db.as_ref(), &request).await
    }

    pub async fn mutate_request_for_tests(
        &self,
        request_id: &str,
        op: impl FnOnce(&mut Request),
    ) -> Result<(), ApiError> {
        let mut request = request_by_id(self.db.as_ref(), request_id)
            .await?
            .ok_or_else(|| ApiError::not_found("test request not found"))?;
        op(&mut request);
        save_request_row(self.db.as_ref(), &request).await
    }

    pub async fn user_for_tests(&self, user_id: &str) -> Result<Option<UserAccount>, ApiError> {
        entities::user::Entity::find_by_id(user_id.to_string())
            .one(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?
            .map(entities::user::Model::try_into_domain)
            .transpose()
    }

    pub async fn repository_for_tests(
        &self,
        repo_id: &str,
    ) -> Result<Option<StoredRepository>, ApiError> {
        let row = entities::repository::Entity::find_by_id(repo_id.to_string())
            .one(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?;
        match row {
            Some(row) => repository_from_model(self.db.as_ref(), row).await.map(Some),
            None => Ok(None),
        }
    }

    pub async fn request_for_tests(&self, request_id: &str) -> Result<Option<Request>, ApiError> {
        request_by_id(self.db.as_ref(), request_id).await
    }

    pub async fn request_events_for_tests(&self) -> Result<Vec<RequestEvent>, ApiError> {
        entities::request_event::Entity::find()
            .all(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?
            .into_iter()
            .map(entities::request_event::Model::try_into_domain)
            .collect()
    }

    pub async fn credit_account_for_tests(
        &self,
        user_id: &str,
    ) -> Result<Option<UserCreditAccount>, ApiError> {
        entities::user_credit_account::Entity::find_by_id(user_id.to_string())
            .one(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?
            .map(entities::user_credit_account::Model::try_into_domain)
            .transpose()
    }

    pub async fn credit_ledger_entries_for_tests(
        &self,
    ) -> Result<Vec<CreditLedgerEntry>, ApiError> {
        entities::credit_ledger_entry::Entity::find()
            .all(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?
            .into_iter()
            .map(entities::credit_ledger_entry::Model::try_into_domain)
            .collect()
    }

    pub async fn user_count_for_tests(&self) -> Result<u64, ApiError> {
        use sea_orm::PaginatorTrait;
        entities::user::Entity::find()
            .count(self.db.as_ref())
            .await
            .map_err(ApiError::internal)
    }

    pub async fn repository_count_for_tests(&self) -> Result<u64, ApiError> {
        use sea_orm::PaginatorTrait;
        entities::repository::Entity::find()
            .count(self.db.as_ref())
            .await
            .map_err(ApiError::internal)
    }
}

#[cfg(any(test, feature = "test-support"))]
async fn ensure_repository_users_for_tests<C>(
    conn: &C,
    repo: &StoredRepository,
) -> Result<(), ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let users = std::iter::once((
        repo.record.owner_user_id.as_str(),
        repo.record.owner_handle.as_str(),
    ))
    .chain(
        repo.members
            .iter()
            .map(|member| (member.user_id.as_str(), member.user_id.as_str())),
    );
    for (id, handle) in users {
        if entities::user::Entity::find_by_id(id.to_string())
            .one(conn)
            .await
            .map_err(ApiError::internal)?
            .is_none()
        {
            entities::user::Model::from_domain(&UserAccount {
                id: id.to_string(),
                handle: handle.to_string(),
                email: format!("{id}@scope.test"),
                email_verified: true,
            })
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
        }
    }
    Ok(())
}

#[cfg(feature = "local-dev")]
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
    for change_block in catalog.request_change_blocks.values() {
        insert_change_block(&tx, change_block).await?;
    }
    for discussion in catalog.request_discussions.values() {
        insert_discussion(&tx, discussion).await?;
    }
    for reply in catalog.request_discussion_replies.values() {
        insert_reply(&tx, reply).await?;
    }
    for read_state in catalog.request_discussion_read_states.values() {
        save_read_state(&tx, read_state).await?;
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
            .or_insert_with(|| UserAccount {
                id: id.clone(),
                handle,
                email: format!("{id}@scope.test"),
                email_verified: true,
            });
    }
}

#[cfg(any(test, feature = "test-support"))]
fn validate_test_database_url(database_url: &str) -> anyhow::Result<()> {
    let lower = database_url.trim().to_ascii_lowercase();
    if !(lower.starts_with("postgres://") || lower.starts_with("postgresql://")) {
        anyhow::bail!("SCOPE_TEST_DATABASE_URL must be a postgres:// or postgresql:// URL");
    }

    let target = lower
        .split_once("://")
        .and_then(|(_, rest)| rest.split_once('/').map(|(_, path)| path))
        .unwrap_or_default();
    let database_name = target.split(['?', '#']).next().unwrap_or_default();
    let query = target
        .split_once('?')
        .map(|(_, query)| query.split('#').next().unwrap_or_default())
        .unwrap_or_default();
    let has_test_marker = has_scope_test_marker(database_name)
        || query
            .split('&')
            .filter_map(|part| part.split_once('='))
            .any(|(key, value)| {
                matches!(
                    key,
                    "search_path" | "schema" | "current_schema" | "currentschema"
                ) && has_scope_test_marker(value)
            });

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
    fn test_database_url_accepts_only_explicit_postgres_test_targets() {
        for url in [
            "postgres://localhost/scope_test",
            "postgres://localhost/scope-vcs-test",
            "postgres://localhost/postgres?search_path=scope_test_run",
        ] {
            validate_test_database_url(url).unwrap();
        }
        for url in [
            "postgres://localhost/scope_staging",
            "postgres://localhost/prod?application_name=scope_test",
            "postgres://localhost/prod?foo=scope_test",
            "sqlite://scope_test",
        ] {
            assert!(
                validate_test_database_url(url).is_err(),
                "{url} must be rejected"
            );
        }
    }
}
