use super::{
    GeneratedIdSource, RepositoryStore, acquire_aggregate_lock,
    cleanup_queue::{
        claim::claim_pending_repo_storage_cleanup,
        completion::complete_claimed_repo_storage_cleanup,
        queue::{pending_repo_storage_cleanup_exists, queue_pending_source_blob_deletion_rows},
    },
    entities,
    object_references::delete_repository_object_references,
    repo_effects::save_repo_effects,
    repository_from_model,
    repository_rows::insert_repository,
    request_revision_rows::revisions_for_request_ids,
    request_rows::requests_by_repo_id,
};
use crate::error::PostgresError;
use scope_domain::{
    content::SourceBlob,
    policy::Visibility,
    repo_actions::{create_repo as create_repo_command, delete_repo as delete_repo_command},
    repository::credentials::{FirstPushToken, GitPushToken},
    repository::{Repository, repo_id},
    requests::Request,
};
use sea_orm::sea_query::Query;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, Statement,
    TransactionTrait,
};
use std::{collections::BTreeSet, sync::Arc};

#[cfg(test)]
use super::MetadataStore;

#[derive(Debug)]
pub enum RepositoryCreationError<E> {
    Cleanup(E),
    Persistence(PostgresError),
}

impl<E> From<PostgresError> for RepositoryCreationError<E> {
    fn from(error: PostgresError) -> Self {
        Self::Persistence(error)
    }
}

pub struct CreateRepositoryCommand {
    pub owner_user_id: String,
    pub name: String,
    pub default_visibility: Visibility,
    pub init_tokens: (FirstPushToken, GitPushToken),
    pub now_unix: u64,
}

impl RepositoryStore {
    pub async fn create_repo_with_init_tokens<F, E>(
        &self,
        command: CreateRepositoryCommand,
        generated_ids: &dyn GeneratedIdSource,
        cleanup_pending_storage: F,
    ) -> Result<Repository, RepositoryCreationError<E>>
    where
        F: FnOnce(&str, &str) -> Result<(), E> + Send + 'static,
        E: Send + 'static,
    {
        let CreateRepositoryCommand {
            owner_user_id,
            name,
            default_visibility,
            init_tokens: (first_push_token, git_push_token),
            now_unix,
        } = command;
        let owner = entities::user::Entity::find_by_id(owner_user_id)
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("signed-in user was not persisted"))?
            .try_into_domain()?;
        let mutation = create_repo_command(
            &owner,
            &name,
            default_visibility,
            first_push_token,
            git_push_token,
        )
        .map_err(PostgresError::from)?;
        let repo = mutation.result;
        let db = Arc::clone(&self.db);
        let repo_id = repo.record.id.clone();
        self.with_repo_storage_lock(&repo_id, move || async move {
            let claim_tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
            acquire_aggregate_lock(&claim_tx, "repository", &repo.record.id).await?;
            ensure_repository_absent(&claim_tx, &repo.record.id).await?;
            let cleanup_claim = claim_pending_repo_storage_cleanup(
                &claim_tx,
                &repo.record.id,
                now_unix,
                generated_ids,
            )
            .await?;
            claim_tx.commit().await.map_err(PostgresError::internal)?;

            if cleanup_claim.is_some() {
                cleanup_pending_storage(&repo.record.owner_handle, &repo.record.name)
                    .map_err(RepositoryCreationError::Cleanup)?;
            }

            let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
            acquire_aggregate_lock(&tx, "repository", &repo.record.id).await?;
            ensure_repository_absent(&tx, &repo.record.id).await?;
            match cleanup_claim {
                Some(claim) => {
                    complete_claimed_repo_storage_cleanup(&tx, &repo.record.id, &claim, now_unix)
                        .await?
                }
                None if pending_repo_storage_cleanup_exists(&tx, &repo.record.id).await? => {
                    return Err(PostgresError::conflict(
                        "repository storage cleanup changed during creation; retry",
                    )
                    .into());
                }
                None => {}
            }

            insert_repository(&tx, &repo, now_unix, generated_ids).await?;
            save_repo_effects(&tx, &mutation.effects, now_unix, generated_ids).await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            Ok(repo)
        })
        .await
    }

    pub async fn delete_repo(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<String, PostgresError> {
        let repo_id = repo_id(owner, name);
        let owner = owner.to_string();
        let name = name.to_string();
        let user_id = user_id.to_string();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let repo = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| scope_domain::repo_actions::hidden_repo_not_found(&owner, &name))?;
        let repo = repository_from_model(&tx, repo).await?;
        let mutation = delete_repo_command(&repo, &user_id, &owner, &name)?;
        let requests = lock_requests_for_repo_postgres(&tx, &repo_id).await?;
        let request_ids = requests
            .iter()
            .map(|request| request.id.clone())
            .collect::<Vec<_>>();
        let revisions = revisions_for_request_ids(&tx, &request_ids).await?;
        let mut retained_sources = request_git_snapshots_for_repo(&requests, &revisions);
        let revision_ids = revisions
            .iter()
            .map(|revision| revision.id.clone())
            .collect::<Vec<_>>();
        delete_repository_object_references(&tx, &repo_id, &request_ids, &revision_ids).await?;
        let runs = entities::run::Entity::find()
            .filter(entities::run::Column::RepoId.eq(repo_id.clone()))
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::run::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        let run_ids = runs.iter().map(|run| run.id.clone()).collect::<Vec<_>>();
        let workflow_digests = runs
            .iter()
            .map(|run| run.workflow_revision_digest.clone())
            .collect::<BTreeSet<_>>();
        retained_sources.extend(
            runs.iter()
                .flat_map(|run| run.source.retained_objects())
                .cloned(),
        );
        if !run_ids.is_empty() {
            entities::object_reference::Entity::delete_many()
                .filter(entities::object_reference::Column::RefKind.eq("run_source"))
                .filter(entities::object_reference::Column::RefId.is_in(run_ids))
                .exec(&tx)
                .await
                .map_err(PostgresError::internal)?;
        }

        entities::repository_invite::Entity::delete_many()
            .filter(entities::repository_invite::Column::RepoId.eq(repo_id.clone()))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::repository_member::Entity::delete_many()
            .filter(entities::repository_member::Column::RepoId.eq(repo_id.clone()))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::git_pack_span::Entity::delete_many()
            .filter(entities::git_pack_span::Column::RepoId.eq(repo_id.clone()))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM scope_git_segment_references refs
             USING scope_git_segment_uploads uploads
             WHERE refs.segment_id = uploads.segment_id AND uploads.repo_id = $1",
            [repo_id.clone().into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_git_segment_uploads
             SET state = 'deleting', updated_at_unix = GREATEST(updated_at_unix, $2)
             WHERE repo_id = $1 AND state IN ('uploading', 'ready', 'published')",
            [
                repo_id.clone().into(),
                i64::try_from(now_unix)
                    .map_err(|_| {
                        PostgresError::internal_message(
                            "repository deletion time exceeds database bigint",
                        )
                    })?
                    .into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        entities::repository::Entity::delete_by_id(repo_id.clone())
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        if !workflow_digests.is_empty() {
            entities::workflow_revision::Entity::delete_many()
                .filter(entities::workflow_revision::Column::Digest.is_in(workflow_digests))
                .filter(
                    entities::workflow_revision::Column::Digest.not_in_subquery(
                        Query::select()
                            .column(entities::run::Column::WorkflowRevisionDigest)
                            .from(entities::run::Entity)
                            .to_owned(),
                    ),
                )
                .exec(&tx)
                .await
                .map_err(PostgresError::internal)?;
        }

        save_repo_effects(&tx, &mutation.effects, now_unix, generated_ids).await?;
        queue_pending_source_blob_deletion_rows(&tx, retained_sources, now_unix, generated_ids)
            .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation.result)
    }
}

async fn ensure_repository_absent<C>(conn: &C, repo_id: &str) -> Result<(), PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    if entities::repository::Entity::find_by_id(repo_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .is_some()
    {
        Err(PostgresError::conflict(format!(
            "repo {repo_id} already exists"
        )))
    } else {
        Ok(())
    }
}

async fn lock_requests_for_repo_postgres<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Vec<Request>, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let mut request_ids = requests_by_repo_id(conn, repo_id)
        .await?
        .into_iter()
        .map(|request| request.id)
        .collect::<Vec<_>>();
    request_ids.sort();

    let mut requests = Vec::with_capacity(request_ids.len());
    for request_id in request_ids {
        acquire_aggregate_lock(conn, "request", &request_id).await?;
        if let Some(request) = super::request_rows::request_by_id(conn, &request_id).await?
            && request.repo_id == repo_id
        {
            requests.push(request);
        }
    }
    Ok(requests)
}

fn request_git_snapshots_for_repo(
    requests: &[Request],
    revisions: &[scope_domain::requests::RequestRevision],
) -> Vec<SourceBlob> {
    let mut snapshots = requests
        .iter()
        .filter_map(|request| request.git_snapshot.clone())
        .chain(
            revisions
                .iter()
                .map(|revision| revision.git_snapshot.clone()),
        )
        .collect::<Vec<_>>();
    snapshots.sort_by(|left, right| left.content_ref.cmp(&right.content_ref));
    snapshots.dedup_by(|left, right| left.content_ref == right.content_ref);
    snapshots
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::{
        account::UserAccount,
        repo_actions::RepoStorageCleanup,
        repository::credentials::{FirstPushToken, GitPushToken},
    };
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };

    fn test_init_tokens(owner_user_id: &str) -> (FirstPushToken, GitPushToken) {
        static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
        let suffix = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
        let now = 1_700_000_000;
        (
            FirstPushToken {
                token_hash: format!("test-first-push-{suffix}"),
                secret: Some(format!("test-first-push-secret-{suffix}")),
                owner_user_id: owner_user_id.to_string(),
                created_at_unix: now,
                expires_at_unix: now + 300,
                used_at_unix: None,
            },
            GitPushToken {
                token_hash: format!("test-git-push-{suffix}"),
                owner_user_id: owner_user_id.to_string(),
                created_at_unix: now,
            },
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pending_storage_cleanup_reserves_repo_name_until_recreation_commits() {
        let target = super::super::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let mut catalog = crate::db::CatalogFixture::default();
        catalog.users.insert(
            "user_owner".to_string(),
            UserAccount {
                id: "user_owner".to_string(),
                handle: "owner".to_string(),
                email: "owner@example.com".to_string(),
                email_verified: true,
            },
        );
        store.admin().seed_catalog_for_tests(catalog).unwrap();
        super::super::cleanup_queue::queue::queue_pending_repo_storage_cleanup_row(
            store.db.as_ref(),
            RepoStorageCleanup {
                owner_handle: "owner".to_string(),
                repo_name: "repo".to_string(),
            },
            1_700_000_000,
            &super::super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();

        let (cleanup_started_tx, cleanup_started_rx) = tokio::sync::oneshot::channel();
        let (release_cleanup_tx, release_cleanup_rx) = std::sync::mpsc::channel();
        let first_store = store.clone();
        let first = tokio::spawn(async move {
            let (first_push_token, git_push_token) = test_init_tokens("user_owner");
            first_store
                .repositories()
                .create_repo_with_init_tokens(
                    CreateRepositoryCommand {
                        owner_user_id: "user_owner".to_string(),
                        name: "repo".to_string(),
                        default_visibility: Visibility::Private,
                        init_tokens: (first_push_token, git_push_token),
                        now_unix: 1_700_000_000,
                    },
                    &super::super::generated_ids::test_generated_id,
                    move |_, _| {
                        cleanup_started_tx.send(()).unwrap();
                        release_cleanup_rx.recv().unwrap();
                        Ok::<(), PostgresError>(())
                    },
                )
                .await
        });
        cleanup_started_rx.await.unwrap();

        let competing_cleanup_called = Arc::new(AtomicBool::new(false));
        let second_cleanup_called = Arc::clone(&competing_cleanup_called);
        let second_store = store.clone();
        let second = tokio::spawn(async move {
            let (first_push_token, git_push_token) = test_init_tokens("user_owner");
            second_store
                .repositories()
                .create_repo_with_init_tokens(
                    CreateRepositoryCommand {
                        owner_user_id: "user_owner".to_string(),
                        name: "repo".to_string(),
                        default_visibility: Visibility::Private,
                        init_tokens: (first_push_token, git_push_token),
                        now_unix: 1_700_000_000,
                    },
                    &super::super::generated_ids::test_generated_id,
                    move |_, _| {
                        second_cleanup_called.store(true, Ordering::SeqCst);
                        Ok::<(), PostgresError>(())
                    },
                )
                .await
        });
        wait_until_repository_lock_is_waited_on(&store).await;
        assert!(
            !second.is_finished(),
            "competing creator must wait for the storage path lock"
        );
        release_cleanup_tx.send(()).unwrap();
        let first_result = first.await.unwrap();
        let second_result = second.await.unwrap();

        first_result.unwrap();
        assert!(matches!(
            second_result.unwrap_err(),
            RepositoryCreationError::Persistence(error) if error.message.contains("already exists")
        ));
        assert!(!competing_cleanup_called.load(Ordering::SeqCst));
        assert!(
            entities::repository::Entity::find_by_id("owner/repo")
                .one(store.db.as_ref())
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn expired_creation_cleanup_claim_cannot_commit_after_worker_reclaims_it() {
        let target = super::super::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        super::super::cleanup_queue::queue::queue_pending_repo_storage_cleanup_row(
            store.db.as_ref(),
            RepoStorageCleanup {
                owner_handle: "owner".to_string(),
                repo_name: "repo".to_string(),
            },
            1_700_000_000,
            &super::super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();

        let claim_tx = store.db.begin().await.unwrap();
        acquire_aggregate_lock(&claim_tx, "repository", "owner/repo")
            .await
            .unwrap();
        let claim = claim_pending_repo_storage_cleanup(
            &claim_tx,
            "owner/repo",
            1_700_000_000,
            &super::super::generated_ids::test_generated_id,
        )
        .await
        .unwrap()
        .unwrap();
        claim_tx.commit().await.unwrap();

        entities::repo_storage_cleanup_job::Entity::update_many()
            .filter(entities::repo_storage_cleanup_job::Column::RepoId.eq("owner/repo"))
            .col_expr(
                entities::repo_storage_cleanup_job::Column::NextRunAtUnix,
                sea_orm::sea_query::Expr::value(0_i64),
            )
            .exec(store.db.as_ref())
            .await
            .unwrap();
        let _worker_batch = store
            .cleanup()
            .repo_storage_cleanup_batch(
                1_700_000_000,
                &super::super::generated_ids::test_generated_id,
            )
            .await
            .unwrap();

        let create_tx = store.db.begin().await.unwrap();
        acquire_aggregate_lock(&create_tx, "repository", "owner/repo")
            .await
            .unwrap();
        let error =
            complete_claimed_repo_storage_cleanup(&create_tx, "owner/repo", &claim, 1_700_000_000)
                .await
                .unwrap_err();
        assert!(error.message.contains("changed during creation"));
    }

    async fn wait_until_repository_lock_is_waited_on(store: &MetadataStore) {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let waiting = store
                    .db
                    .query_one(Statement::from_string(
                        DatabaseBackend::Postgres,
                        "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND wait_event = 'advisory') AS waiting".to_string(),
                    ))
                    .await
                    .unwrap()
                    .unwrap()
                    .try_get::<bool>("", "waiting")
                    .unwrap();
                if waiting {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("competing creator should wait for repository lock");
    }
}
