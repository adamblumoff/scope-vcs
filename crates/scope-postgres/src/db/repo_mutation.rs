use super::{
    GeneratedIdSource, RepositoryStore, acquire_aggregate_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows, entities,
    push_triggers::enqueue_push_main_trigger_evaluation, repository_from_model,
    repository_rows::save_repository_delta,
};
use sea_orm::{EntityTrait, TransactionTrait};
use std::{fmt, sync::Arc};
use {
    crate::error::PostgresError,
    scope_domain::error::DomainError,
    scope_domain::store::{SourceBlob, StoredRepository, repo_id},
};

#[derive(Debug)]
pub enum RepositoryMutationError {
    Behavior(DomainError),
    Persistence(PostgresError),
}

impl fmt::Display for RepositoryMutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Behavior(error) => error.fmt(formatter),
            Self::Persistence(error) => formatter.write_str(&error.message),
        }
    }
}

impl std::error::Error for RepositoryMutationError {}

impl From<DomainError> for RepositoryMutationError {
    fn from(error: DomainError) -> Self {
        Self::Behavior(error)
    }
}

impl From<PostgresError> for RepositoryMutationError {
    fn from(error: PostgresError) -> Self {
        Self::Persistence(error)
    }
}

pub struct RepositoryMutation<R> {
    pub result: R,
    pub orphan_objects: Vec<SourceBlob>,
    pub push_trigger_input: Option<scope_domain::runs::trigger::PushTriggerInput>,
}

impl<R> RepositoryMutation<R> {
    pub fn new(result: R) -> Self {
        Self {
            result,
            orphan_objects: Vec::new(),
            push_trigger_input: None,
        }
    }

    pub fn with_source_blob_deletions(result: R, orphan_objects: Vec<SourceBlob>) -> Self {
        Self {
            result,
            orphan_objects,
            push_trigger_input: None,
        }
    }

    pub fn with_push_trigger_input(
        result: R,
        push_trigger_input: scope_domain::runs::trigger::PushTriggerInput,
    ) -> Self {
        Self {
            result,
            orphan_objects: Vec::new(),
            push_trigger_input: Some(push_trigger_input),
        }
    }
}

impl RepositoryStore {
    pub async fn mutate_repository<R, F>(
        &self,
        owner: &str,
        name: &str,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
        op: F,
    ) -> Result<R, RepositoryMutationError>
    where
        R: Send + 'static,
        F: FnOnce(&mut StoredRepository) -> Result<RepositoryMutation<R>, DomainError>
            + Send
            + 'static,
    {
        let repo_id = repo_id(owner, name);
        let owner = owner.to_string();
        let name = name.to_string();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let repo = entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
        let mut repo = repository_from_model(&tx, repo).await?;
        let before = repo.clone();
        let mutation = op(&mut repo)?;
        save_repository_delta(&tx, &before, &repo, now_unix, generated_ids).await?;
        if let Some(input) = mutation.push_trigger_input {
            let head = repo.git_head.as_ref().ok_or_else(|| {
                PostgresError::internal_message(
                    "push trigger evaluation requires an accepted Git head",
                )
            })?;
            enqueue_push_main_trigger_evaluation(
                &tx,
                &repo.record.id,
                head,
                &input,
                now_unix,
                generated_ids,
            )
            .await?;
        }
        if !mutation.orphan_objects.is_empty() {
            queue_pending_source_blob_deletion_rows(
                &tx,
                mutation.orphan_objects,
                now_unix,
                generated_ids,
            )
            .await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation.result)
    }
}
