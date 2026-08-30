use super::{
    GeneratedIdSource, RepositoryStore, acquire_aggregate_lock,
    cleanup_queue::queue::queue_pending_source_blob_deletion_rows, entities,
    landing_files::apply_repository_landing_file_mutation,
    push_triggers::enqueue_push_main_trigger_evaluation, repository_from_model,
    repository_rows::save_repository_delta, workflow_catalogs::apply_repository_workflow_catalog,
};
use sea_orm::{EntityTrait, TransactionTrait};
use std::{fmt, sync::Arc, time::Instant};
use {
    crate::error::PostgresError,
    scope_domain::content::SourceBlob,
    scope_domain::repository::{Repository, repo_id},
    scope_domain::runs::catalog::RepositoryWorkflowCatalog,
    scope_domain::{error::DomainError, landing_file::RepositoryLandingFileMutation},
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
    pub landing_file_mutation: RepositoryLandingFileMutation,
    pub workflow_catalog: Option<RepositoryWorkflowCatalog>,
}

impl<R> RepositoryMutation<R> {
    pub fn new(result: R) -> Self {
        Self {
            result,
            orphan_objects: Vec::new(),
            push_trigger_input: None,
            landing_file_mutation: RepositoryLandingFileMutation::Unchanged,
            workflow_catalog: None,
        }
    }

    pub fn with_source_blob_deletions(result: R, orphan_objects: Vec<SourceBlob>) -> Self {
        Self {
            result,
            orphan_objects,
            push_trigger_input: None,
            landing_file_mutation: RepositoryLandingFileMutation::Unchanged,
            workflow_catalog: None,
        }
    }

    pub fn with_push_trigger_input(
        result: R,
        push_trigger_input: scope_domain::runs::trigger::PushTriggerInput,
        landing_file_mutation: RepositoryLandingFileMutation,
        workflow_catalog: RepositoryWorkflowCatalog,
    ) -> Self {
        Self {
            result,
            orphan_objects: Vec::new(),
            push_trigger_input: Some(push_trigger_input),
            landing_file_mutation,
            workflow_catalog: Some(workflow_catalog),
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
        F: FnOnce(&mut Repository) -> Result<RepositoryMutation<R>, DomainError> + Send + 'static,
    {
        let repo_id = repo_id(owner, name);
        let owner = owner.to_string();
        let name = name.to_string();
        let db = Arc::clone(&self.db);
        let transaction_started = Instant::now();
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let lock_started = Instant::now();
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let lock_wait = lock_started.elapsed();
        let serialized_started = Instant::now();
        let repository_row_started = Instant::now();
        let repo = entities::repository::Entity::find_by_id(&repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
        let repository_row_us = repository_row_started.elapsed().as_micros();
        let hydrate_started = Instant::now();
        let mut repo = repository_from_model(&tx, repo).await?;
        let hydrate_us = hydrate_started.elapsed().as_micros();
        let clone_started = Instant::now();
        let before = repo.clone();
        let clone_us = clone_started.elapsed().as_micros();
        let domain_apply_started = Instant::now();
        let mutation = op(&mut repo)?;
        let domain_apply_us = domain_apply_started.elapsed().as_micros();
        let changed_file_count = repo
            .graph
            .commits
            .last()
            .map_or(0, |commit| commit.changes.len());
        let live_file_count = repo.live_files.len();
        let logical_commit_count = repo.graph.commits.len();
        let visibility_change_set_count = repo.visibility_change_sets.len();
        let policy_rule_count = repo.policy.rules().len();
        let config_rule_count = repo.repo_config.visibility.rules.len();
        let catalog_verify_started = Instant::now();
        if let Some(catalog) = &mutation.workflow_catalog {
            let head = repo.git_head.as_ref().ok_or_else(|| {
                PostgresError::internal_message(
                    "repository workflow catalog requires an accepted Git head",
                )
            })?;
            catalog
                .verify_source(&repo.record.id, &head.head_oid, head.change_version)
                .map_err(PostgresError::internal)?;
        }
        let catalog_verify_us = catalog_verify_started.elapsed().as_micros();
        let save_delta_started = Instant::now();
        save_repository_delta(&tx, &before, &repo, now_unix, generated_ids).await?;
        let save_delta_us = save_delta_started.elapsed().as_micros();
        let landing_file_started = Instant::now();
        apply_repository_landing_file_mutation(
            &tx,
            &repo.record.id,
            mutation.landing_file_mutation,
        )
        .await?;
        let landing_file_us = landing_file_started.elapsed().as_micros();
        let workflow_catalog_started = Instant::now();
        if let Some(catalog) = &mutation.workflow_catalog {
            apply_repository_workflow_catalog(&tx, catalog).await?;
        }
        let workflow_catalog_us = workflow_catalog_started.elapsed().as_micros();
        let push_trigger_started = Instant::now();
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
                &repo.git_pack_spans,
                &input,
                now_unix,
                generated_ids,
            )
            .await?;
        }
        let push_trigger_us = push_trigger_started.elapsed().as_micros();
        let orphan_queue_started = Instant::now();
        if !mutation.orphan_objects.is_empty() {
            queue_pending_source_blob_deletion_rows(
                &tx,
                mutation.orphan_objects,
                now_unix,
                generated_ids,
            )
            .await?;
        }
        let orphan_queue_us = orphan_queue_started.elapsed().as_micros();
        let commit_started = Instant::now();
        tx.commit().await.map_err(PostgresError::internal)?;
        tracing::info!(
            repository_id = repo_id,
            protocol = "aggregate-mutation",
            changed_file_count,
            live_file_count,
            logical_commit_count,
            visibility_change_set_count,
            policy_rule_count,
            config_rule_count,
            lock_wait_us = lock_wait.as_micros(),
            repository_row_us,
            hydrate_us,
            clone_us,
            domain_apply_us,
            catalog_verify_us,
            save_delta_us,
            landing_file_us,
            workflow_catalog_us,
            push_trigger_us,
            orphan_queue_us,
            body_us = commit_started
                .duration_since(serialized_started)
                .as_micros(),
            serialized_us = serialized_started.elapsed().as_micros(),
            commit_us = commit_started.elapsed().as_micros(),
            total_us = transaction_started.elapsed().as_micros(),
            "repository mutation persistence timing"
        );
        Ok(mutation.result)
    }
}
