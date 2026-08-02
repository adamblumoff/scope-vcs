//! Metadata persistence entry point.
//!
//! Table row shapes live in `entities/*`, while ordered schema transitions live
//! in `migrations/*`. Runtime behavior stays in the focused DB modules that own
//! the workflow being persisted.

mod auth;
#[cfg(any(test, feature = "local-dev", feature = "test-support"))]
mod catalog_fixture;
#[cfg(any(test, feature = "local-dev", feature = "test-support"))]
pub use catalog_fixture::CatalogFixture;
mod cli_auth_results;
pub use cli_auth_results::{
    BrowserLoginCompletion, CliSessionSummary, CreateCliExchangeGrantCommand, DeviceLoginPoll,
    NewCliSession, StartBrowserLoginCommand, StartDeviceLoginCommand,
};
mod cleanup_queue;
#[cfg(test)]
mod cleanup_queue_tests;
mod clerk_users;
mod cli_auth;
mod cli_sessions;
mod content_push_transactions;
mod entities;
mod fast_push;
mod generated_ids;
mod git_compaction;
#[cfg(test)]
mod migration_harness_tests;
#[cfg(test)]
mod migration_tests;
#[cfg(test)]
mod runner_protocol_cutover_migration_tests;
pub use generated_ids::{GeneratedIdKind, GeneratedIdSource};
mod git_push_reads;
mod history_rows;
mod locks;
mod object_references;
mod outbox;
mod projection_encoding;
mod projection_read_models;
mod push_triggers;
mod repo_change_notifications;
mod repo_collaboration;
mod repo_effects;
mod repo_lifecycle;
mod repo_mutation;
mod repo_reads;
mod repository_rows;
mod request_access;
mod request_change_block_rows;
mod request_discussion_rows;
pub use request_discussion_rows::RequestDiscussionReplyReadModel;
mod request_discussions;
pub use request_discussions::{
    RequestDiscussionReadBatch, RequestDiscussionReadModel, RequestDiscussionsPageQuery,
};
mod request_invitees;
pub use request_invitees::{
    AddRequestInviteeCommand, LeaveRequestCommand, RemoveRequestInviteeCommand, RequestInviteeRead,
};
mod request_queue;
pub use request_queue::{RequestQueueCursor, RequestQueuePageQuery, RequestQueueRow};
mod request_rows;
pub use request_rows::RequestListRow;
mod request_identity_transactions;
mod request_merge;
mod request_review_transactions;
mod request_revision_transactions;
mod requests;
mod run_attempt_persistence;
mod run_details;
mod run_operations;
mod run_retention;
mod run_step_operations;
mod runner_protocol_cutover;
#[cfg(test)]
mod runner_protocol_cutover_tests;
mod runs;
pub use run_details::{RunAttemptDetail, RunDetail, StoredAttemptStepLogs};
pub use run_operations::{RecentRunLogs, RepositoryRunner};
pub use runner_protocol_cutover::RunnerProtocolCutoverSnapshot;
pub use runs::{DispatchClaim, StoredRunLog};
#[cfg(test)]
mod run_concurrency_tests;
#[cfg(test)]
mod runs_tests;
#[cfg(any(test, feature = "local-dev", feature = "test-support"))]
mod test_support;
mod visibility_changes;

use crate::error::PostgresError;
#[cfg(any(test, feature = "test-support"))]
pub use clerk_users::scope_user_id_for_auth_identity;
pub use fast_push::ApplyContentOnlyPushCommand;
pub use git_compaction::GitCompactionCandidate;
pub use git_push_reads::GitPushContext;
use history_rows::load_repository_histories;
use locks::acquire_aggregate_lock;
pub use outbox::{OutboxJobCounts, OutboxRunSummary};
pub use repo_collaboration::{
    CreateRepositoryInviteMutation, UpdateRepositoryMemberPermissionsCommand,
};
pub use repo_lifecycle::{CreateRepositoryCommand, RepositoryCreationError};
pub use repo_mutation::{RepositoryMutation, RepositoryMutationError};
pub use repo_reads::RepoSummaryRead;
use repository_rows::load_repository_facts;
use scope_domain::store::{RepositoryInvite, RepositoryMember, StoredRepository, repo_id};
use sea_orm::{
    AccessMode, ColumnTrait, ConnectionTrait, Database, DatabaseConnection, DatabaseTransaction,
    EntityTrait, IsolationLevel, QueryFilter, QueryOrder, TransactionTrait,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{sync::Arc, time::Duration};
#[cfg(any(test, feature = "test-support"))]
pub use test_support::TestDatabaseTarget;
pub use visibility_changes::UpdateRepoFileVisibilityCommand;

#[derive(Clone)]
pub struct MetadataStore {
    db: Arc<DatabaseConnection>,
    postgres_database_url: Option<Arc<str>>,
    #[cfg(any(test, feature = "test-support"))]
    _test_schema: Option<Arc<test_support::TestSchemaLease>>,
}

#[derive(Clone)]
pub struct JobStore {
    db: Arc<DatabaseConnection>,
}

#[derive(Clone)]
pub struct AdminStore {
    db: Arc<DatabaseConnection>,
}

#[derive(Clone)]
pub struct AuthStore {
    db: Arc<DatabaseConnection>,
}

#[derive(Clone)]
pub struct CleanupStore {
    db: Arc<DatabaseConnection>,
}

#[derive(Clone)]
pub struct RepositoryStore {
    db: Arc<DatabaseConnection>,
    postgres_database_url: Option<Arc<str>>,
}

#[derive(Clone)]
pub struct RequestStore {
    db: Arc<DatabaseConnection>,
}

#[derive(Clone)]
pub struct RunStore {
    db: Arc<DatabaseConnection>,
}

impl MetadataStore {
    pub fn admin(&self) -> AdminStore {
        AdminStore {
            db: Arc::clone(&self.db),
        }
    }

    pub fn auth(&self) -> AuthStore {
        AuthStore {
            db: Arc::clone(&self.db),
        }
    }

    pub fn cleanup(&self) -> CleanupStore {
        CleanupStore {
            db: Arc::clone(&self.db),
        }
    }

    pub fn repositories(&self) -> RepositoryStore {
        RepositoryStore {
            db: Arc::clone(&self.db),
            postgres_database_url: self.postgres_database_url.clone(),
        }
    }

    pub fn requests(&self) -> RequestStore {
        RequestStore {
            db: Arc::clone(&self.db),
        }
    }

    pub fn jobs(&self) -> JobStore {
        JobStore {
            db: Arc::clone(&self.db),
        }
    }

    pub fn runs(&self) -> RunStore {
        RunStore {
            db: Arc::clone(&self.db),
        }
    }

    pub async fn connect(database_url: String) -> anyhow::Result<Self> {
        connect_postgres_store(database_url).await
    }

    pub async fn connect_worker_with_schema_wait(
        database_url: String,
        wait_timeout: Duration,
        retry_interval: Duration,
    ) -> anyhow::Result<Self> {
        connect_postgres_worker_store_with_schema_wait(database_url, wait_timeout, retry_interval)
            .await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn connect_fresh_for_tests(target: &TestDatabaseTarget) -> anyhow::Result<Self> {
        test_support::connect_postgres_test_store(target)
    }
}

impl RepositoryStore {
    pub async fn repository(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<StoredRepository>, PostgresError> {
        let id = repo_id(owner, name);
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let repo = match entities::repository::Entity::find_by_id(id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        {
            Some(repo) => Some(repository_from_model(&tx, repo).await?),
            None => None,
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(repo)
    }
}

impl AdminStore {
    pub async fn readiness_check(&self) -> Result<(), PostgresError> {
        crate::migrations::assert_exact_state(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)
    }
}

async fn connect_postgres_store(database_url: String) -> anyhow::Result<MetadataStore> {
    let database_url = Arc::<str>::from(database_url);
    let connect_database_url = database_url.to_string();
    let db = Database::connect(&connect_database_url).await?;
    crate::migrations::apply(&db).await?;
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
    crate::migrations::assert_exact_state(&db).await?;
    Ok(db)
}

pub(super) async fn begin_metadata_read_snapshot(
    db: &DatabaseConnection,
) -> Result<DatabaseTransaction, PostgresError> {
    db.begin_with_config(
        Some(IsolationLevel::RepeatableRead),
        Some(AccessMode::ReadOnly),
    )
    .await
    .map_err(PostgresError::internal)
}

async fn repositories_from_models<C>(
    conn: &C,
    repositories: Vec<entities::repository::Model>,
) -> Result<Vec<StoredRepository>, PostgresError>
where
    C: ConnectionTrait,
{
    let repo_ids = repositories
        .iter()
        .map(|repo| repo.id.clone())
        .collect::<Vec<_>>();
    let mut facts_by_repo = load_repository_facts(conn, &repo_ids).await?;
    let mut histories_by_repo = load_repository_histories(conn, &repo_ids).await?;
    let members = if repo_ids.is_empty() {
        Vec::new()
    } else {
        entities::repository_member::Entity::find()
            .filter(entities::repository_member::Column::RepoId.is_in(repo_ids.clone()))
            .order_by_asc(entities::repository_member::Column::RepoId)
            .order_by_asc(entities::repository_member::Column::UserId)
            .all(conn)
            .await
            .map_err(PostgresError::internal)?
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
            .map_err(PostgresError::internal)?
    };
    let members_by_repo = members.into_iter().try_fold(
        std::collections::BTreeMap::<String, Vec<RepositoryMember>>::new(),
        |mut by_repo, member| {
            let repo_id = member.repo_id.clone();
            by_repo
                .entry(repo_id)
                .or_default()
                .push(member.try_into_domain()?);
            Ok::<_, PostgresError>(by_repo)
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
            Ok::<_, PostgresError>(by_repo)
        },
    )?;

    repositories
        .into_iter()
        .map(|repo| {
            let repo_id = repo.id.clone();
            let members = members_by_repo.get(&repo_id).cloned().unwrap_or_default();
            let invitations = invites_by_repo.get(&repo_id).cloned().unwrap_or_default();
            let facts = facts_by_repo.remove(&repo_id).ok_or_else(|| {
                PostgresError::internal_message(format!("repository facts missing for {repo_id}"))
            })?;
            let history = histories_by_repo.remove(&repo_id).ok_or_else(|| {
                PostgresError::internal_message(format!("repository history missing for {repo_id}"))
            })?;
            repo.try_into_domain(facts.into_facts(), members, invitations, history)
        })
        .collect()
}

async fn repository_from_model<C>(
    conn: &C,
    repository: entities::repository::Model,
) -> Result<StoredRepository, PostgresError>
where
    C: ConnectionTrait,
{
    repositories_from_models(conn, vec![repository])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| PostgresError::internal_message("repository row disappeared while loading"))
}

fn encode_json<T: Serialize>(value: &T) -> Result<serde_json::Value, PostgresError> {
    serde_json::to_value(value).map_err(PostgresError::internal)
}

fn decode_json<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, PostgresError> {
    serde_json::from_value(value).map_err(PostgresError::internal)
}
