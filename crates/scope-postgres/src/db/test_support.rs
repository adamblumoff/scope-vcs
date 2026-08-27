use super::{
    AdminStore, CatalogFixture, acquire_aggregate_lock,
    cleanup_queue::queue::{
        save_pending_repo_storage_deletions, save_pending_source_blob_deletions,
    },
    entities,
    repository_rows::insert_repository,
    request_discussion_rows::{insert_discussion, insert_reply, save_read_state},
    request_revision_rows::insert_revision,
    request_rows::{insert_request_event_row, insert_request_row},
    workflow_catalogs::apply_repository_workflow_catalog,
};
#[cfg(any(test, feature = "test-support"))]
use super::{
    AuthStore, CleanupStore, MetadataStore, RepositoryStore, RequestStore,
    cleanup_queue::queue::{
        load_pending_repo_storage_deletions, load_pending_source_blob_deletions,
        queue_pending_repo_storage_cleanup_row_at,
    },
    repository_from_model,
    repository_rows::save_repository_delta,
    request_rows::{request_by_id, save_request_row},
};
#[cfg(any(test, feature = "test-support"))]
use scope_domain::{
    content::SourceBlob,
    repo_actions::RepoStorageCleanup,
    requests::{Request, RequestEvent},
};
use sea_orm::{ActiveModelTrait, ConnectionTrait, IntoActiveModel, TransactionTrait};
#[cfg(any(test, feature = "test-support"))]
use sea_orm::{ConnectOptions, Database, DatabaseConnection, EntityTrait, Statement};
#[cfg(any(test, feature = "test-support"))]
use std::{
    future::Future,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use {
    crate::error::PostgresError,
    scope_domain::{
        account::UserAccount, repository::Repository, runs::catalog::RepositoryWorkflowCatalog,
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

    #[cfg(test)]
    pub(super) fn schema_database_url(&self) -> String {
        let separator = if self.database_url.contains('?') {
            '&'
        } else {
            '?'
        };
        format!(
            "{}{separator}options[search_path]={}",
            self.database_url, self.schema_name
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn connect_postgres_test_store(target: &TestDatabaseTarget) -> anyhow::Result<MetadataStore> {
    let postgres_database_url = Arc::from(target.database_url.clone());
    let target = target.clone();
    let (db, test_schema) = run_test_future(async move {
        let (db, test_schema) = connect_isolated_test_database(&target).await?;
        crate::migrations::apply_in_maintenance(db.as_ref()).await?;
        Ok::<_, anyhow::Error>((db, test_schema))
    })?;

    Ok(MetadataStore {
        db,
        postgres_database_url: Some(postgres_database_url),
        _test_schema: Some(test_schema),
    })
}

#[cfg(any(test, feature = "test-support"))]
pub(super) async fn connect_isolated_test_database(
    target: &TestDatabaseTarget,
) -> anyhow::Result<(Arc<DatabaseConnection>, Arc<TestSchemaLease>)> {
    let admin = Database::connect(&target.database_url).await?;
    admin
        .execute(Statement::from_string(
            admin.get_database_backend(),
            format!(
                "CREATE SCHEMA IF NOT EXISTS {}",
                quote_pg_ident(&target.schema_name)
            ),
        ))
        .await?;

    let mut options = ConnectOptions::new(target.database_url.clone());
    options
        .max_connections(8)
        .min_connections(1)
        .set_schema_search_path(target.schema_name.clone());
    let db = Arc::new(Database::connect(options).await?);
    let test_schema = Arc::new(TestSchemaLease {
        database: Arc::clone(&db),
        database_url: target.database_url.clone(),
        schema_name: target.schema_name.clone(),
    });
    Ok((db, test_schema))
}

impl AdminStore {
    #[cfg(any(feature = "local-dev", feature = "smoke-seed"))]
    pub async fn replace_catalog_for_seed(
        &self,
        catalog: CatalogFixture,
    ) -> Result<(), PostgresError> {
        replace_catalog(self.db.as_ref(), catalog).await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn seed_catalog_for_tests(&self, catalog: CatalogFixture) -> Result<(), PostgresError> {
        let db = Arc::clone(&self.db);
        run_test_future(async move { seed_catalog(db.as_ref(), catalog).await })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl CleanupStore {
    pub async fn queue_repo_storage_cleanup_for_tests(
        &self,
        cleanup: RepoStorageCleanup,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        queue_pending_repo_storage_cleanup_row_at(
            self.db.as_ref(),
            cleanup,
            now_unix,
            &super::generated_ids::test_generated_id,
        )
        .await
    }

    pub async fn pending_repo_storage_cleanups_for_tests(
        &self,
    ) -> Result<Vec<RepoStorageCleanup>, PostgresError> {
        load_pending_repo_storage_deletions(self.db.as_ref()).await
    }

    pub async fn pending_source_blob_cleanups_for_tests(
        &self,
    ) -> Result<Vec<SourceBlob>, PostgresError> {
        load_pending_source_blob_deletions(self.db.as_ref()).await
    }
}

const CATALOG_SEED_NOW_UNIX: u64 = 1_700_000_000;

#[cfg(any(test, feature = "test-support"))]
impl AuthStore {
    pub async fn insert_user_for_tests(&self, user: UserAccount) -> Result<(), PostgresError> {
        entities::user::Model::from_domain(&user)
            .into_active_model()
            .insert(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
impl RepositoryStore {
    pub async fn replace_repository_for_tests(
        &self,
        repo: Repository,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        ensure_repository_users_for_tests(&tx, &repo).await?;
        acquire_aggregate_lock(&tx, "repository", &repo.record.id).await?;
        match entities::repository::Entity::find_by_id(repo.record.id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        {
            Some(row) => {
                let before = repository_from_model(&tx, row).await?;
                save_repository_delta(
                    &tx,
                    &before,
                    &repo,
                    CATALOG_SEED_NOW_UNIX,
                    &super::generated_ids::test_generated_id,
                )
                .await?;
            }
            None => {
                insert_repository(
                    &tx,
                    &repo,
                    CATALOG_SEED_NOW_UNIX,
                    &super::generated_ids::test_generated_id,
                )
                .await?
            }
        }
        tx.commit().await.map_err(PostgresError::internal)
    }

    pub async fn mutate_repository_for_tests(
        &self,
        repo_id: &str,
        op: impl FnOnce(&mut scope_domain::repository::Repository),
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", repo_id).await?;
        let row = entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("test repository not found"))?;
        let mut repo = repository_from_model(&tx, row).await?;
        let before = repo.clone();
        op(&mut repo);
        save_repository_delta(
            &tx,
            &before,
            &repo,
            CATALOG_SEED_NOW_UNIX,
            &super::generated_ids::test_generated_id,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl AuthStore {
    pub async fn user_for_tests(
        &self,
        user_id: &str,
    ) -> Result<Option<UserAccount>, PostgresError> {
        entities::user::Entity::find_by_id(user_id.to_string())
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .map(entities::user::Model::try_into_domain)
            .transpose()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl RepositoryStore {
    pub async fn repository_for_tests(
        &self,
        repo_id: &str,
    ) -> Result<Option<Repository>, PostgresError> {
        let row = entities::repository::Entity::find_by_id(repo_id.to_string())
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        match row {
            Some(row) => repository_from_model(self.db.as_ref(), row).await.map(Some),
            None => Ok(None),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl AuthStore {
    pub async fn user_count_for_tests(&self) -> Result<u64, PostgresError> {
        use sea_orm::PaginatorTrait;
        entities::user::Entity::find()
            .count(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl RepositoryStore {
    pub async fn repository_count_for_tests(&self) -> Result<u64, PostgresError> {
        use sea_orm::PaginatorTrait;
        entities::repository::Entity::find()
            .count(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl RequestStore {
    pub async fn insert_request_for_tests(&self, request: Request) -> Result<(), PostgresError> {
        insert_request_row(self.db.as_ref(), &request).await
    }

    pub async fn mutate_request_for_tests(
        &self,
        request_id: &str,
        op: impl FnOnce(&mut Request),
    ) -> Result<(), PostgresError> {
        let mut request = request_by_id(self.db.as_ref(), request_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("test request not found"))?;
        op(&mut request);
        save_request_row(self.db.as_ref(), &request).await
    }

    pub async fn request_for_tests(
        &self,
        request_id: &str,
    ) -> Result<Option<Request>, PostgresError> {
        request_by_id(self.db.as_ref(), request_id).await
    }

    pub async fn request_events_for_tests(&self) -> Result<Vec<RequestEvent>, PostgresError> {
        entities::request_event::Entity::find()
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::request_event::Model::try_into_domain)
            .collect()
    }
}

#[cfg(any(test, feature = "test-support"))]
async fn ensure_repository_users_for_tests<C>(
    conn: &C,
    repo: &Repository,
) -> Result<(), PostgresError>
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
            .map_err(PostgresError::internal)?
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
            .map_err(PostgresError::internal)?;
        }
    }
    Ok(())
}

#[cfg(any(feature = "local-dev", feature = "smoke-seed"))]
async fn replace_catalog(
    db: &sea_orm::DatabaseConnection,
    catalog: CatalogFixture,
) -> Result<(), PostgresError> {
    let tx = db.begin().await.map_err(PostgresError::internal)?;
    let tables = tx
        .query_one(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "
                SELECT string_agg(
                    format('%I.%I', schemaname, tablename),
                    ', ' ORDER BY tablename
                ) AS tables
                FROM pg_tables
                WHERE schemaname = current_schema()
                  AND left(tablename, 6) = 'scope_'
            "
            .to_string(),
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("PostgreSQL did not list catalog tables"))?
        .try_get::<String>("", "tables")
        .map_err(PostgresError::internal)?;
    tx.execute_unprepared(&format!("TRUNCATE TABLE {tables} RESTART IDENTITY CASCADE"))
        .await
        .map_err(PostgresError::internal)?;
    seed_catalog_rows(&tx, catalog).await?;
    tx.commit().await.map_err(PostgresError::internal)
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

#[cfg(any(test, feature = "test-support"))]
async fn seed_catalog(
    conn: &sea_orm::DatabaseConnection,
    catalog: CatalogFixture,
) -> Result<(), PostgresError> {
    let tx = conn.begin().await.map_err(PostgresError::internal)?;
    seed_catalog_rows(&tx, catalog).await?;
    tx.commit().await.map_err(PostgresError::internal)
}

async fn seed_catalog_rows(
    tx: &sea_orm::DatabaseTransaction,
    mut catalog: CatalogFixture,
) -> Result<(), PostgresError> {
    complete_test_users(&mut catalog);
    acquire_aggregate_lock(tx, "test", "catalog").await?;
    for user in catalog.users.values() {
        entities::user::Model::from_domain(user)
            .into_active_model()
            .insert(tx)
            .await
            .map_err(PostgresError::internal)?;
    }
    for upload in &catalog.git_segment_uploads {
        entities::git_segment_upload::Model::from_domain(upload)?
            .into_active_model()
            .insert(tx)
            .await
            .map_err(PostgresError::internal)?;
    }
    for repo in catalog.repositories.values() {
        insert_repository(
            tx,
            repo,
            CATALOG_SEED_NOW_UNIX,
            &super::generated_ids::test_generated_id,
        )
        .await?;
        seed_empty_repository_workflow_catalog(tx, repo).await?;
    }
    for request in catalog.requests.values() {
        insert_request_row(tx, request).await?;
    }
    for revision in catalog.request_revisions.values() {
        insert_revision(tx, revision).await?;
    }
    for discussion in catalog.request_discussions.values() {
        insert_discussion(tx, discussion).await?;
    }
    for reply in catalog.request_discussion_replies.values() {
        insert_reply(tx, reply).await?;
    }
    for read_state in catalog.request_discussion_read_states.values() {
        save_read_state(tx, read_state).await?;
    }
    for event in catalog.request_events.values() {
        insert_request_event_row(tx, event).await?;
    }
    save_pending_repo_storage_deletions(
        tx,
        &catalog.pending_repo_storage_deletions,
        CATALOG_SEED_NOW_UNIX,
    )
    .await?;
    save_pending_source_blob_deletions(
        tx,
        &catalog.pending_source_blob_deletions,
        CATALOG_SEED_NOW_UNIX,
    )
    .await?;
    Ok(())
}

async fn seed_empty_repository_workflow_catalog(
    tx: &sea_orm::DatabaseTransaction,
    repo: &Repository,
) -> Result<(), PostgresError> {
    let Some(head) = &repo.git_head else {
        return Ok(());
    };
    if repo
        .live_files
        .keys()
        .any(|path| path.as_str().starts_with("/.scope/runs/"))
    {
        return Ok(());
    }
    let catalog = RepositoryWorkflowCatalog::captured(
        &repo.record.id,
        &head.head_oid,
        repo.record.change_version,
        Vec::new(),
    )
    .map_err(PostgresError::internal)?;
    catalog
        .verify_source(&repo.record.id, &head.head_oid, head.change_version)
        .map_err(PostgresError::internal)?;
    apply_repository_workflow_catalog(tx, &catalog).await
}

fn complete_test_users(catalog: &mut CatalogFixture) {
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
    #[cfg(feature = "local-dev")]
    use sea_orm::DatabaseBackend;

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

    #[cfg(feature = "local-dev")]
    #[test]
    fn local_catalog_replacement_preserves_migration_history() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        let db = Arc::clone(&store.db);
        run_test_future(async move {
            db.execute_unprepared(
                "
                    INSERT INTO scope_users (id, handle, email, email_verified)
                    VALUES ('user_old', 'old', 'old@scope.test', TRUE);
                    CREATE TABLE scopex_private (sentinel text NOT NULL);
                    INSERT INTO scopex_private (sentinel) VALUES ('keep')
                ",
            )
            .await
            .unwrap();
            let before = db
                .query_all(Statement::from_string(
                    DatabaseBackend::Postgres,
                    "SELECT version FROM seaql_migrations ORDER BY version".to_string(),
                ))
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.try_get::<String>("", "version").unwrap())
                .collect::<Vec<_>>();

            replace_catalog(db.as_ref(), CatalogFixture::default())
                .await
                .unwrap();

            let after = db
                .query_all(Statement::from_string(
                    DatabaseBackend::Postgres,
                    "SELECT version FROM seaql_migrations ORDER BY version".to_string(),
                ))
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.try_get::<String>("", "version").unwrap())
                .collect::<Vec<_>>();
            assert_eq!(after, before);
            let user_count = db
                .query_one(Statement::from_string(
                    DatabaseBackend::Postgres,
                    "SELECT count(*) AS count FROM scope_users".to_string(),
                ))
                .await
                .unwrap()
                .unwrap()
                .try_get::<i64>("", "count")
                .unwrap();
            assert_eq!(user_count, 0);
            let unrelated_sentinel = db
                .query_one(Statement::from_string(
                    DatabaseBackend::Postgres,
                    "SELECT sentinel FROM scopex_private".to_string(),
                ))
                .await
                .unwrap()
                .unwrap()
                .try_get::<String>("", "sentinel")
                .unwrap();
            assert_eq!(unrelated_sentinel, "keep");
        });
    }
}
