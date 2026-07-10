//! Metadata persistence entry point.
//!
//! Table row shapes live in `entities/*`, while destructive pre-alpha DDL lives
//! in `schema.sql`. Runtime behavior should stay in the
//! focused DB modules that own the workflow being persisted.

mod auth;
mod cleanup_queue;
#[cfg(test)]
mod cleanup_queue_tests;
mod clerk_users;
mod cli_auth;
mod cli_sessions;
mod entities;
mod locks;
mod metadata_reset;
mod outbox;
mod projection_read_models;
mod repo_change_notifications;
mod repo_collaboration;
mod repo_effects;
mod repo_lifecycle;
mod repo_mutation;
mod repo_reads;
mod repository_rows;
mod request_access;
mod request_merges;
mod request_rows;
mod requests;
mod schema;
#[cfg(any(test, feature = "local-dev", feature = "test-support"))]
mod test_support;
mod visibility_changes;

#[cfg(any(test, feature = "test-support"))]
use crate::domain::store::AppCatalog;
use crate::domain::store::{RepositoryInvite, RepositoryMember, StoredRepository, repo_id};
use crate::error::ApiError;
#[cfg(any(test, feature = "test-support"))]
pub use clerk_users::scope_user_id_for_auth_identity;
use locks::{acquire_metadata_read_lock, acquire_metadata_write_lock, ensure_metadata_lock_row};
pub use metadata_reset::MetadataResetEvent;
use metadata_reset::{
    insert_metadata_reset_event, metadata_reset_event_from_model, new_operator_metadata_reset_event,
};
pub use outbox::{OutboxJobCounts, OutboxRunSummary};
pub use repo_collaboration::CreateRepositoryInviteMutation;
pub use repo_mutation::RepositoryMutation;
pub use repo_reads::RepoSummaryRead;
use repository_rows::load_repository_facts;
#[cfg(any(test, feature = "test-support"))]
use request_rows::load_request_catalog_rows;
use sea_orm::{
    AccessMode, ColumnTrait, ConnectionTrait, Database, DatabaseConnection, DatabaseTransaction,
    EntityTrait, IsolationLevel, QueryFilter, QueryOrder, Statement, TransactionTrait,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{sync::Arc, time::Duration};
#[cfg(any(test, feature = "test-support"))]
pub use test_support::{TestCatalogGuard, TestDatabaseTarget};

const METADATA_LOCK_KEY: &str = "catalog";

#[derive(Clone)]
pub struct MetadataStore {
    db: Arc<DatabaseConnection>,
    postgres_database_url: Option<Arc<str>>,
    #[cfg(any(test, feature = "test-support"))]
    _test_schema: Option<Arc<test_support::TestSchemaLease>>,
}

impl MetadataStore {
    pub async fn connect_from_env() -> anyhow::Result<Self> {
        let database_url = std::env::var(crate::config::DATABASE_URL_ENV)
            .map_err(|_| anyhow::anyhow!("DATABASE_URL is required for Scope metadata storage"))?;
        connect_postgres_store(database_url).await
    }

    pub async fn connect_worker_from_env_with_schema_wait(
        wait_timeout: Duration,
        retry_interval: Duration,
    ) -> anyhow::Result<Self> {
        let database_url = std::env::var(crate::config::DATABASE_URL_ENV)
            .map_err(|_| anyhow::anyhow!("DATABASE_URL is required for Scope worker metadata"))?;
        connect_postgres_worker_store_with_schema_wait(database_url, wait_timeout, retry_interval)
            .await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn connect_for_tests(target: &TestDatabaseTarget) -> anyhow::Result<Self> {
        test_support::connect_postgres_test_store(target, false)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn connect_fresh_for_tests(target: &TestDatabaseTarget) -> anyhow::Result<Self> {
        test_support::connect_postgres_test_store(target, true)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn read<R>(
        &self,
        op: impl FnOnce(&AppCatalog) -> Result<R, ApiError>,
    ) -> Result<R, ApiError>
    where
        R: Send + 'static,
    {
        let catalog = load_catalog(&self.db).await?;
        op(&catalog)
    }

    pub async fn repository(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<StoredRepository>, ApiError> {
        let id = repo_id(owner, name);
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
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
    }

    pub async fn readiness_check(&self) -> Result<(), ApiError> {
        self.db
            .query_one(Statement::from_string(
                self.db.get_database_backend(),
                "SELECT 1".to_string(),
            ))
            .await
            .map_err(ApiError::internal)?;
        Ok(())
    }

    pub async fn metadata_reset_events(&self) -> Result<Vec<MetadataResetEvent>, ApiError> {
        let events = entities::metadata_reset_event::Entity::find()
            .order_by_desc(entities::metadata_reset_event::Column::ResetAtUnix)
            .order_by_desc(entities::metadata_reset_event::Column::Id)
            .all(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?;
        Ok(events
            .into_iter()
            .map(metadata_reset_event_from_model)
            .collect())
    }

    pub async fn reset_catalog(&self, reason: &str) -> Result<MetadataResetEvent, ApiError> {
        let event = new_operator_metadata_reset_event(reason);
        schema::reset_metadata_schema(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?;
        schema::migrate_metadata_schema(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?;
        ensure_metadata_lock_row(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?;
        insert_metadata_reset_event(self.db.as_ref(), &event)
            .await
            .map_err(ApiError::internal)?;
        Ok(event)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_catalog(&self) -> Result<TestCatalogGuard, ApiError> {
        test_support::test_catalog(self)
    }
}

async fn connect_postgres_store(database_url: String) -> anyhow::Result<MetadataStore> {
    let database_url = Arc::<str>::from(database_url);
    let connect_database_url = database_url.to_string();
    let db = Database::connect(&connect_database_url).await?;
    schema::migrate_metadata_schema(&db).await?;
    ensure_metadata_lock_row(&db).await?;
    Ok(MetadataStore {
        db: Arc::new(db),
        postgres_database_url: Some(database_url),
        #[cfg(any(test, feature = "test-support"))]
        _test_schema: None,
    })
}

async fn connect_postgres_worker_store_with_schema_wait(
    database_url: String,
    wait_timeout: Duration,
    retry_interval: Duration,
) -> anyhow::Result<MetadataStore> {
    let database_url = Arc::<str>::from(database_url);
    let connect_database_url = database_url.to_string();
    let started = tokio::time::Instant::now();
    let db = loop {
        match connect_worker_database_once(&connect_database_url).await {
            Ok(db) => break db,
            Err(error) if started.elapsed() < wait_timeout => {
                tracing::warn!(
                    error = %error,
                    retry_in_secs = retry_interval.as_secs_f64(),
                    "metadata schema is not ready for worker; waiting for API migrations"
                );
                tokio::time::sleep(retry_interval).await;
            }
            Err(error) => return Err(error.into()),
        }
    };

    Ok(MetadataStore {
        db: Arc::new(db),
        postgres_database_url: Some(database_url),
        #[cfg(any(test, feature = "test-support"))]
        _test_schema: None,
    })
}

async fn connect_worker_database_once(
    database_url: &str,
) -> Result<DatabaseConnection, sea_orm::DbErr> {
    let db = Database::connect(database_url).await?;
    schema::assert_metadata_schema_ready(&db).await?;
    Ok(db)
}

#[cfg(any(test, feature = "test-support"))]
async fn load_catalog(db: &DatabaseConnection) -> Result<AppCatalog, ApiError> {
    let tx = begin_metadata_read_snapshot(db).await?;
    let catalog = load_catalog_async(&tx).await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(catalog)
}

pub(super) async fn begin_metadata_read_snapshot(
    db: &DatabaseConnection,
) -> Result<DatabaseTransaction, ApiError> {
    db.begin_with_config(
        Some(IsolationLevel::RepeatableRead),
        Some(AccessMode::ReadOnly),
    )
    .await
    .map_err(ApiError::internal)
}

#[cfg(any(test, feature = "test-support"))]
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
    let members = entities::repository_member::Entity::find()
        .order_by_asc(entities::repository_member::Column::RepoId)
        .order_by_asc(entities::repository_member::Column::UserId)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    let invites = entities::repository_invite::Entity::find()
        .order_by_asc(entities::repository_invite::Column::RepoId)
        .order_by_asc(entities::repository_invite::Column::InvitedEmailNormalized)
        .order_by_asc(entities::repository_invite::Column::Id)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    let pending_repo_storage_deletions =
        cleanup_queue::load_pending_repo_storage_deletions(conn).await?;
    let pending_source_blob_deletions =
        cleanup_queue::load_pending_source_blob_deletions(conn).await?;

    let users = users
        .into_iter()
        .map(|user| Ok((user.id.clone(), user.try_into_domain()?)))
        .collect::<Result<_, ApiError>>()?;
    let members_by_repo = members.into_iter().try_fold(
        std::collections::BTreeMap::<String, Vec<RepositoryMember>>::new(),
        |mut by_repo, member| {
            let repo_id = member.repo_id.clone();
            by_repo
                .entry(repo_id)
                .or_default()
                .push(member.try_into_domain()?);
            Ok::<_, ApiError>(by_repo)
        },
    )?;
    let invites_by_repo = invites.into_iter().try_fold(
        std::collections::BTreeMap::<String, Vec<RepositoryInvite>>::new(),
        |mut by_repo, invite| {
            let repo_id = invite.repo_id.clone();
            by_repo
                .entry(repo_id)
                .or_default()
                .push(invite.try_into_domain()?);
            Ok::<_, ApiError>(by_repo)
        },
    )?;
    let repo_ids = repositories
        .iter()
        .map(|repo| repo.id.clone())
        .collect::<Vec<_>>();
    let mut facts_by_repo = load_repository_facts(conn, &repo_ids).await?;
    let repositories = repositories
        .into_iter()
        .map(|repo| {
            let repo_id = repo.id.clone();
            let members = members_by_repo.get(&repo_id).cloned().unwrap_or_default();
            let invitations = invites_by_repo.get(&repo_id).cloned().unwrap_or_default();
            let facts = facts_by_repo.remove(&repo_id).ok_or_else(|| {
                ApiError::internal_message(format!("repository facts missing for {repo_id}"))
            })?;
            let repo = repo.try_into_domain(facts.into_facts(), members, invitations)?;
            Ok((repo.record.id.clone(), repo))
        })
        .collect::<Result<_, ApiError>>()?;

    let request_rows = load_request_catalog_rows(conn).await?;

    Ok(AppCatalog {
        users,
        repositories,
        requests: request_rows.requests,
        request_events: request_rows.request_events,
        user_credit_accounts: request_rows.user_credit_accounts,
        credit_ledger_entries: request_rows.credit_ledger_entries,
        pending_repo_storage_deletions,
        pending_source_blob_deletions,
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
    let mut facts_by_repo = load_repository_facts(conn, &repo_ids).await?;
    let members = if repo_ids.is_empty() {
        Vec::new()
    } else {
        entities::repository_member::Entity::find()
            .filter(entities::repository_member::Column::RepoId.is_in(repo_ids.clone()))
            .order_by_asc(entities::repository_member::Column::RepoId)
            .order_by_asc(entities::repository_member::Column::UserId)
            .all(conn)
            .await
            .map_err(ApiError::internal)?
    };
    let invites = if repo_ids.is_empty() {
        Vec::new()
    } else {
        entities::repository_invite::Entity::find()
            .filter(entities::repository_invite::Column::RepoId.is_in(repo_ids))
            .order_by_asc(entities::repository_invite::Column::RepoId)
            .order_by_asc(entities::repository_invite::Column::InvitedEmailNormalized)
            .order_by_asc(entities::repository_invite::Column::Id)
            .all(conn)
            .await
            .map_err(ApiError::internal)?
    };
    let members_by_repo = members.into_iter().try_fold(
        std::collections::BTreeMap::<String, Vec<RepositoryMember>>::new(),
        |mut by_repo, member| {
            let repo_id = member.repo_id.clone();
            by_repo
                .entry(repo_id)
                .or_default()
                .push(member.try_into_domain()?);
            Ok::<_, ApiError>(by_repo)
        },
    )?;
    let invites_by_repo = invites.into_iter().try_fold(
        std::collections::BTreeMap::<String, Vec<RepositoryInvite>>::new(),
        |mut by_repo, invite| {
            let repo_id = invite.repo_id.clone();
            by_repo
                .entry(repo_id)
                .or_default()
                .push(invite.try_into_domain()?);
            Ok::<_, ApiError>(by_repo)
        },
    )?;

    repositories
        .into_iter()
        .map(|repo| {
            let repo_id = repo.id.clone();
            let members = members_by_repo.get(&repo_id).cloned().unwrap_or_default();
            let invitations = invites_by_repo.get(&repo_id).cloned().unwrap_or_default();
            let facts = facts_by_repo.remove(&repo_id).ok_or_else(|| {
                ApiError::internal_message(format!("repository facts missing for {repo_id}"))
            })?;
            repo.try_into_domain(facts.into_facts(), members, invitations)
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

fn encode_json<T: Serialize>(value: &T) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(value).map_err(ApiError::internal)
}

fn decode_json<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, ApiError> {
    serde_json::from_value(value).map_err(ApiError::internal)
}
