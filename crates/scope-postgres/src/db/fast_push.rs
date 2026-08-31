use super::{
    GeneratedIdSource, RepositoryStore, acquire_aggregate_lock,
    content_push_transactions::{RepositoryContentSnapshots, accept_and_persist_content_push},
    entities,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use std::time::Instant;
use {
    crate::error::PostgresError,
    scope_domain::{
        landing_file::RepositoryLandingFileMutation,
        repo_config::RepoConfig,
        repository::access::repository_push_policy_for_user_id,
        repository::{RepoLifecycleState, RepositoryIncarnation},
        reviewed_updates::content::{
            ReviewedUpdateAuthorization, ReviewedUpdateInput, authorize_reviewed_update,
        },
        runs::catalog::RepositoryWorkflowCatalog,
    },
};

pub struct ApplyContentOnlyPushCommand {
    pub incarnation: RepositoryIncarnation,
    pub owner: String,
    pub name: String,
    pub author_id: String,
    pub expected_manifest_ref: scope_domain::content_ref::ContentRef,
    pub update: ReviewedUpdateInput,
    pub landing_file_mutation: RepositoryLandingFileMutation,
    pub workflow_catalog: RepositoryWorkflowCatalog,
    pub push_trigger_input: scope_domain::runs::trigger::PushTriggerInput,
    pub now_unix: u64,
}

impl RepositoryStore {
    pub async fn apply_content_only_push(
        &self,
        command: ApplyContentOnlyPushCommand,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<Option<scope_domain::repository::git::GitHead>, PostgresError> {
        let ApplyContentOnlyPushCommand {
            incarnation,
            owner,
            name,
            author_id,
            expected_manifest_ref,
            update,
            landing_file_mutation,
            workflow_catalog,
            push_trigger_input,
            now_unix,
        } = command;
        let repo_id = scope_domain::repository::repo_id(&owner, &name);
        let transaction_started = Instant::now();
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let lock_started = Instant::now();
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let lock_wait = lock_started.elapsed();
        let serialized_started = Instant::now();
        let metadata_started = Instant::now();
        let changed_file_count = update.changes.len();
        let config_rule_count = update.config.visibility.rules.len();
        let repo_row = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
        if repo_row.incarnation_id != incarnation.incarnation_id()
            || repo_row.id != incarnation.repository_id()
        {
            return Err(PostgresError::conflict(
                "repository was recreated since push preparation",
            ));
        }
        let publication_state: RepoLifecycleState = serde_json::from_value(
            serde_json::Value::String(repo_row.publication_state.clone()),
        )
        .map_err(PostgresError::internal)?;
        if publication_state != RepoLifecycleState::Ready {
            return Ok(None);
        }
        let head = entities::git_head::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::conflict("repo has no accepted Git head"))?
            .try_into_domain()?;
        if head.manifest.content_ref != expected_manifest_ref {
            return Err(PostgresError::conflict(
                "repo changed since push was reviewed; rerun scope push",
            ));
        }
        let member_permissions = entities::repository_member::Entity::find()
            .filter(entities::repository_member::Column::RepoId.eq(repo_id.clone()))
            .filter(entities::repository_member::Column::UserId.eq(author_id.clone()))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .map(entities::repository_member::Model::try_into_domain)
            .transpose()?
            .map(|member| member.permissions);
        let push_policy = repository_push_policy_for_user_id(
            &repo_row.owner_user_id,
            publication_state,
            member_permissions,
            &author_id,
        );
        let current_config: RepoConfig = serde_json::from_value(repo_row.repo_config.clone())
            .map_err(PostgresError::internal)?;
        let config_changed = current_config != update.config;
        authorize_reviewed_update(ReviewedUpdateAuthorization {
            access: push_policy.access,
            push_mode: push_policy.mode,
            current_config: &current_config,
            proposed_config: &update.config,
        })?;
        let metadata_us = metadata_started.elapsed().as_micros();
        let (git_head, persistence) = accept_and_persist_content_push(
            &tx,
            repo_row,
            update,
            RepositoryContentSnapshots {
                landing_file_mutation,
                workflow_catalog,
            },
            push_trigger_input,
            now_unix,
            generated_ids,
        )
        .await?;
        let commit_started = Instant::now();
        tx.commit().await.map_err(PostgresError::internal)?;
        tracing::info!(
            repository_id = repo_id,
            protocol = "focused-content-push",
            changed_file_count,
            config_rule_count,
            config_changed,
            lock_wait_us = lock_wait.as_micros(),
            metadata_us,
            load_live_files_us = persistence.load_live_files_us,
            load_previous_commit_us = persistence.load_previous_commit_us,
            load_git_head_us = persistence.load_git_head_us,
            domain_apply_us = persistence.domain_apply_us,
            repository_facts_us = persistence.repository_facts_us,
            load_pack_spans_us = persistence.load_pack_spans_us,
            history_rows_us = persistence.history_rows_us,
            live_file_rows_us = persistence.live_file_rows_us,
            landing_file_us = persistence.landing_file_us,
            workflow_catalog_us = persistence.workflow_catalog_us,
            projection_us = persistence.projection_us,
            push_trigger_us = persistence.push_trigger_us,
            body_us = commit_started
                .duration_since(serialized_started)
                .as_micros(),
            serialized_us = serialized_started.elapsed().as_micros(),
            commit_us = commit_started.elapsed().as_micros(),
            total_us = transaction_started.elapsed().as_micros(),
            "Git push persistence timing"
        );
        Ok(Some(git_head))
    }
}
