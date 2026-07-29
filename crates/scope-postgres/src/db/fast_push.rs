use super::{
    GeneratedIdSource, RepositoryStore, acquire_aggregate_lock,
    content_push_transactions::accept_and_persist_content_push, entities,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use {
    crate::error::PostgresError,
    scope_domain::{
        reviewed_updates::ReviewedUpdateInput,
        store::{
            MainPushMode, RepoPublicationState, RepositoryActor, repository_push_policy_for_user_id,
        },
    },
};

pub struct ApplyContentOnlyPushCommand {
    pub owner: String,
    pub name: String,
    pub author_id: String,
    pub expected_manifest_ref: scope_domain::content_ref::ContentRef,
    pub update: ReviewedUpdateInput,
    pub push_trigger_input: scope_domain::runs::trigger::PushTriggerInput,
    pub now_unix: u64,
}

impl RepositoryStore {
    pub async fn apply_content_only_push(
        &self,
        command: ApplyContentOnlyPushCommand,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<Option<scope_domain::store::GitHead>, PostgresError> {
        let ApplyContentOnlyPushCommand {
            owner,
            name,
            author_id,
            expected_manifest_ref,
            update,
            push_trigger_input,
            now_unix,
        } = command;
        let repo_id = scope_domain::store::repo_id(&owner, &name);
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let repo_row = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
        let publication_state: RepoPublicationState = serde_json::from_value(
            serde_json::Value::String(repo_row.publication_state.clone()),
        )
        .map_err(PostgresError::internal)?;
        if publication_state != RepoPublicationState::Published {
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
        if push_policy.mode != MainPushMode::Published {
            let message = if push_policy.access.actor == RepositoryActor::Public {
                "repo membership required"
            } else {
                "push permission required"
            };
            return Err(PostgresError::permission_denied(message));
        }
        let git_head = accept_and_persist_content_push(
            &tx,
            &repo_id,
            repo_row,
            update,
            push_trigger_input,
            now_unix,
            generated_ids,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(git_head))
    }
}
