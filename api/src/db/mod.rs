mod auth;
mod cleanup_queue;
mod entities;
mod locks;
mod metadata_reset;
mod publish_apply;
mod push_staging;
mod repo_effects;
mod repo_lifecycle;
mod repo_settings;
mod repo_tokens;
mod repository_rows;
mod runtime;
mod schema;
#[cfg(test)]
mod test_support;
mod visibility_changes;

use crate::domain::store::{AppCatalog, RepoMembership, StoredRepository, repo_id};
use crate::error::ApiError;
#[cfg(test)]
pub(crate) use auth::scope_user_id_for_auth_identity;
use locks::{acquire_metadata_read_lock, acquire_metadata_write_lock, ensure_metadata_lock_row};
pub(crate) use metadata_reset::MetadataResetEvent;
use metadata_reset::{
    insert_metadata_reset_event, metadata_reset_event_from_model,
    new_operator_metadata_reset_event, reset_stale_pre_alpha_metadata,
};
use runtime::DbRuntime;
use runtime::{run_api_db_on, run_db_on};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, Database, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, Statement, TransactionTrait,
};
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
pub(crate) use test_support::TestDatabaseTarget;

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
    auth: std::sync::Mutex<auth::MemoryAuthState>,
    reset_events: std::sync::Mutex<Vec<MetadataResetEvent>>,
    fail_next_persist: AtomicBool,
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
                auth: std::sync::Mutex::new(auth::MemoryAuthState::default()),
                reset_events: std::sync::Mutex::new(Vec::new()),
                fail_next_persist: AtomicBool::new(false),
            }))),
        }
    }

    #[cfg(test)]
    pub(crate) fn connect_for_tests(target: &TestDatabaseTarget) -> anyhow::Result<Self> {
        test_support::connect_postgres_test_store(target, false)
    }

    #[cfg(test)]
    pub(crate) fn connect_fresh_for_tests(target: &TestDatabaseTarget) -> anyhow::Result<Self> {
        test_support::connect_postgres_test_store(target, true)
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

    pub(crate) fn repository(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<StoredRepository>, ApiError> {
        let id = repo_id(owner, name);
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_read_lock(&tx).await?;
                    let repo = match entities::repository::Entity::find_by_id(id)
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                    {
                        Some(repo) => Some(repository_from_model(&tx, repo).await?),
                        None => None,
                    };
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(repo)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(catalog) => {
                let catalog = catalog
                    .catalog
                    .lock()
                    .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?;
                Ok(catalog.repositories.get(&id).cloned())
            }
        }
    }

    pub(crate) fn repositories_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<StoredRepository>, ApiError> {
        let user_id = user_id.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_read_lock(&tx).await?;
                    let memberships = entities::membership::Entity::find()
                        .filter(entities::membership::Column::UserId.eq(user_id))
                        .order_by_asc(entities::membership::Column::RepoId)
                        .all(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    let repo_ids = memberships
                        .into_iter()
                        .map(|membership| membership.repo_id)
                        .collect::<Vec<_>>();
                    if repo_ids.is_empty() {
                        tx.commit().await.map_err(ApiError::internal)?;
                        return Ok(Vec::new());
                    }
                    let repositories = entities::repository::Entity::find()
                        .filter(entities::repository::Column::Id.is_in(repo_ids))
                        .order_by_asc(entities::repository::Column::Id)
                        .all(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    let repositories = repositories_from_models(&tx, repositories).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(repositories)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(catalog) => {
                let catalog = catalog
                    .catalog
                    .lock()
                    .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?;
                Ok(catalog
                    .repositories_for_user(&user_id)
                    .into_iter()
                    .cloned()
                    .collect())
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

    pub(crate) fn metadata_reset_events(&self) -> Result<Vec<MetadataResetEvent>, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let events = entities::metadata_reset_event::Entity::find()
                        .order_by_desc(entities::metadata_reset_event::Column::ResetAtUnix)
                        .order_by_desc(entities::metadata_reset_event::Column::Id)
                        .all(db.as_ref())
                        .await
                        .map_err(ApiError::internal)?;
                    Ok(events
                        .into_iter()
                        .map(metadata_reset_event_from_model)
                        .collect())
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut events = memory
                    .reset_events
                    .lock()
                    .map_err(|_| ApiError::internal_message("reset event lock is poisoned"))?
                    .clone();
                events.sort_by(|left, right| {
                    right
                        .reset_at_unix
                        .cmp(&left.reset_at_unix)
                        .then_with(|| right.id.cmp(&left.id))
                });
                Ok(events)
            }
        }
    }

    pub(crate) fn reset_catalog(&self, reason: &str) -> Result<MetadataResetEvent, ApiError> {
        let event = new_operator_metadata_reset_event(reason);
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                let event_to_insert = event.clone();
                run_api_db_on(runtime, async move {
                    schema::reset_metadata_schema(db.as_ref())
                        .await
                        .map_err(ApiError::internal)?;
                    schema::migrate_metadata_schema(db.as_ref())
                        .await
                        .map_err(ApiError::internal)?;
                    ensure_metadata_lock_row(db.as_ref())
                        .await
                        .map_err(ApiError::internal)?;
                    insert_metadata_reset_event(db.as_ref(), &event_to_insert)
                        .await
                        .map_err(ApiError::internal)?;
                    Ok(())
                })?;
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                *memory
                    .catalog
                    .lock()
                    .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))? =
                    AppCatalog::default();
                *memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))? =
                    auth::MemoryAuthState::default();
                memory
                    .reset_events
                    .lock()
                    .map_err(|_| ApiError::internal_message("reset event lock is poisoned"))?
                    .push(event.clone());
            }
        }
        Ok(event)
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

fn connect_postgres_store(database_url: String) -> anyhow::Result<MetadataStore> {
    let runtime = DbRuntime::new()?;
    let db = run_db_on(&runtime, async move {
        let db = Database::connect(&database_url).await?;
        schema::migrate_metadata_schema(&db).await?;
        ensure_metadata_lock_row(&db).await?;
        reset_stale_pre_alpha_metadata(&db).await?;
        Ok::<_, sea_orm::DbErr>(db)
    })?;

    Ok(MetadataStore {
        inner: Arc::new(MetadataStoreInner::Postgres {
            db: Arc::new(db),
            runtime,
        }),
    })
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

async fn repositories_from_models<C>(
    conn: &C,
    repositories: Vec<entities::repository::Model>,
) -> Result<Vec<StoredRepository>, ApiError>
where
    C: ConnectionTrait,
{
    let repo_ids = repositories
        .iter()
        .map(|repo| repo.id.clone())
        .collect::<Vec<_>>();
    let memberships = if repo_ids.is_empty() {
        Vec::new()
    } else {
        entities::membership::Entity::find()
            .filter(entities::membership::Column::RepoId.is_in(repo_ids))
            .order_by_asc(entities::membership::Column::RepoId)
            .order_by_asc(entities::membership::Column::UserId)
            .all(conn)
            .await
            .map_err(ApiError::internal)?
    };
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

    repositories
        .into_iter()
        .map(|repo| {
            let memberships = memberships_by_repo
                .get(&repo.id)
                .cloned()
                .unwrap_or_default();
            repo.try_into_domain(memberships)
        })
        .collect()
}

async fn repository_from_model<C>(
    conn: &C,
    repository: entities::repository::Model,
) -> Result<StoredRepository, ApiError>
where
    C: ConnectionTrait,
{
    repositories_from_models(conn, vec![repository])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::internal_message("repository row disappeared while loading"))
}

async fn save_catalog_async<C>(conn: &C, catalog: &AppCatalog) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let auth_identities = entities::auth_identity::Entity::find()
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    let cli_device_logins = entities::cli_device_login::Entity::find()
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    let cli_access_sessions = entities::cli_access_session::Entity::find()
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
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

    entities::cli_access_session::Entity::delete_many()
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    entities::cli_device_login::Entity::delete_many()
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    entities::auth_identity::Entity::delete_many()
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
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
    for auth_identity in auth_identities {
        auth_identity
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }
    for cli_device_login in cli_device_logins {
        cli_device_login
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }
    for cli_access_session in cli_access_sessions {
        cli_access_session
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

fn encode_json<T: Serialize>(value: &T) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(value).map_err(ApiError::internal)
}

fn decode_json<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, ApiError> {
    serde_json::from_value(value).map_err(ApiError::internal)
}
