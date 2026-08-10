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
#[cfg(test)]
mod file_visibility_migration_tests;
mod generated_ids;
mod git_compaction;
#[cfg(test)]
mod migration_harness_tests;
#[cfg(test)]
mod migration_tests;
#[cfg(test)]
mod request_activity_migration_tests;
#[cfg(test)]
mod request_submission_migration_tests;
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
mod request_discussion_rows;
mod request_revision_rows;
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
mod request_ratings;
mod request_rows;
pub use request_rows::{RequestListPageQuery, RequestListRow};
mod request_merge;
mod request_submission_transactions;
mod requests;
mod run_attempt_mutations;
mod run_attempt_persistence;
mod run_details;
mod run_dispatch;
mod run_history;
mod run_log_reads;
mod run_log_writes;
mod run_operations;
mod run_retention;
mod run_step_operations;
mod runner_protocol_cutover;
#[cfg(test)]
mod runner_protocol_cutover_tests;
mod runs;
pub use run_details::{RunAttemptDetail, RunDetail};
pub use run_history::{RepositoryRun, RunHistoryCursor, RunHistoryPageQuery};
pub use run_log_reads::{RecentRunLogs, StoredAttemptStepLogs, StoredRunLog};
pub use run_operations::RepositoryRunner;
pub use runner_protocol_cutover::RunnerProtocolCutoverSnapshot;
pub use runs::{DispatchClaim, UpgradeRunnerRegistrationCommand};
#[cfg(test)]
mod run_concurrency_tests;
#[cfg(test)]
mod runs_tests;
#[cfg(any(test, feature = "local-dev", feature = "test-support"))]
mod test_support;
mod visibility_changes;

use crate::error::PostgresError;
pub use crate::migrations::{MigrationImpact, MigrationPlan, PendingMigration};
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
    AccessMode, ColumnTrait, ConnectOptions, ConnectionTrait, Database, DatabaseBackend,
    DatabaseConnection, DatabaseTransaction, EntityTrait, IsolationLevel, QueryFilter, QueryOrder,
    SqlxPostgresConnector, Statement, TransactionTrait,
};
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;
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

    pub async fn connect_worker(database_url: String) -> anyhow::Result<Self> {
        connect_postgres_worker_store(database_url).await
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
    let setup_db = Database::connect(&connect_database_url).await?;
    crate::migrations::apply_online(&setup_db).await?;
    setup_db.close().await?;
    let db = connect_writer_database(&connect_database_url).await?;
    crate::migrations::assert_exact_state(&db).await?;
    Ok(MetadataStore {
        db: Arc::new(db),
        postgres_database_url: Some(database_url),
        #[cfg(any(test, feature = "test-support"))]
        _test_schema: None,
    })
}

async fn connect_postgres_worker_store(database_url: String) -> anyhow::Result<MetadataStore> {
    let database_url = Arc::<str>::from(database_url);
    let connect_database_url = database_url.to_string();
    let setup_db = Database::connect(&connect_database_url).await?;
    crate::migrations::assert_exact_state(&setup_db).await?;
    setup_db.close().await?;
    let db = connect_writer_database(&connect_database_url).await?;
    crate::migrations::assert_exact_state(&db).await?;

    Ok(MetadataStore {
        db: Arc::new(db),
        postgres_database_url: Some(database_url),
        #[cfg(any(test, feature = "test-support"))]
        _test_schema: None,
    })
}

const WRITER_FENCE_KEY: &str = "scope:metadata-writers";

pub async fn migration_plan(database_url: String) -> anyhow::Result<MigrationPlan> {
    let db = Database::connect(database_url).await?;
    Ok(crate::migrations::plan(&db).await?)
}

pub async fn verify_schema(database_url: String) -> anyhow::Result<()> {
    let db = Database::connect(database_url).await?;
    crate::migrations::assert_exact_state(&db).await?;
    Ok(())
}

pub async fn apply_maintenance_migrations(database_url: String) -> anyhow::Result<()> {
    let fence = ExclusiveWriterFence::acquire(&database_url).await?;
    let db = Database::connect(database_url).await?;
    let migration_result = crate::migrations::apply_in_maintenance(&db).await;
    let release_result = fence.release().await;
    migration_result?;
    release_result?;
    Ok(())
}

pub struct ExclusiveWriterFence {
    connection: DatabaseConnection,
}

impl ExclusiveWriterFence {
    pub async fn acquire(database_url: &str) -> anyhow::Result<Self> {
        let connection = connect_single_session(database_url).await?;
        let acquired = connection
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                writer_fence_statement("pg_try_advisory_lock"),
            ))
            .await?
            .ok_or_else(|| anyhow::anyhow!("PostgreSQL did not report writer-fence state"))?
            .try_get::<bool>("", "acquired")?;
        if !acquired {
            anyhow::bail!(
                "exclusive writer fence refused: an API or worker writer still holds the database fence"
            );
        }
        Ok(Self { connection })
    }

    pub async fn release(self) -> anyhow::Result<()> {
        self.connection
            .execute_unprepared(&writer_fence_statement("pg_advisory_unlock"))
            .await?;
        self.connection.close().await?;
        Ok(())
    }
}

async fn connect_writer_database(database_url: &str) -> anyhow::Result<DatabaseConnection> {
    let mut options = ConnectOptions::new(database_url.to_string());
    options.min_connections(1);
    let fence_statement = writer_fence_statement("pg_advisory_lock_shared");
    let pool = options
        .sqlx_pool_options()
        .after_connect(move |connection, _| {
            let fence_statement = fence_statement.clone();
            Box::pin(async move {
                sqlx::query(&fence_statement)
                    .execute(connection)
                    .await
                    .map(|_| ())
            })
        })
        .connect(database_url)
        .await?;
    Ok(SqlxPostgresConnector::from_sqlx_postgres_pool(pool))
}

async fn connect_single_session(database_url: &str) -> anyhow::Result<DatabaseConnection> {
    let mut options = ConnectOptions::new(database_url.to_string());
    options.max_connections(1).min_connections(1);
    Ok(Database::connect(options).await?)
}

fn writer_fence_statement(function: &str) -> String {
    format!(
        "SELECT {function}(
            hashtextextended(
                '{WRITER_FENCE_KEY}:' || current_database() || ':' || current_schema(),
                0
            )
        ) AS acquired"
    )
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
