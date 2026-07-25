use super::{
    MetadataStore, acquire_aggregate_lock,
    content_push_transactions::accept_and_persist_content_push, entities,
};
use crate::{
    domain::{
        reviewed_updates::ReviewedUpdateInput,
        store::{
            MainPushMode, RepoPublicationState, RepositoryActor, repository_push_policy_for_user_id,
        },
    },
    error::ApiError,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};

impl MetadataStore {
    pub async fn apply_content_only_push(
        &self,
        owner: &str,
        name: &str,
        author_id: &str,
        expected_manifest_key: &str,
        update: ReviewedUpdateInput,
    ) -> Result<Option<crate::domain::store::GitHead>, ApiError> {
        let repo_id = crate::domain::store::repo_id(owner, name);
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let repo_row = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
        let publication_state: RepoPublicationState = serde_json::from_value(
            serde_json::Value::String(repo_row.publication_state.clone()),
        )
        .map_err(ApiError::internal)?;
        if publication_state != RepoPublicationState::Published {
            return Ok(None);
        }
        let head = entities::git_head::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::conflict("repo has no accepted Git head"))?
            .try_into_domain()?;
        if head.manifest.object_key != expected_manifest_key {
            return Err(ApiError::conflict(
                "repo changed since push was reviewed; rerun scope push",
            ));
        }
        let member_permissions = entities::repository_member::Entity::find()
            .filter(entities::repository_member::Column::RepoId.eq(repo_id.clone()))
            .filter(entities::repository_member::Column::UserId.eq(author_id.to_string()))
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .map(entities::repository_member::Model::try_into_domain)
            .transpose()?
            .map(|member| member.permissions);
        let push_policy = repository_push_policy_for_user_id(
            &repo_row.owner_user_id,
            publication_state,
            member_permissions,
            author_id,
        );
        if push_policy.mode != MainPushMode::Published {
            let message = if push_policy.access.actor == RepositoryActor::Public {
                "repo membership required"
            } else {
                "push permission required"
            };
            return Err(ApiError::forbidden(message));
        }
        let git_head = accept_and_persist_content_push(&tx, &repo_id, repo_row, update).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(Some(git_head))
    }
}
