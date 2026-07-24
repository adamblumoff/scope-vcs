//! Atomic repository content merge plus request assessment and settlement.

use super::{
    MetadataStore, acquire_aggregate_lock,
    content_push_transactions::accept_and_persist_request_merge,
    entities,
    request_access::ensure_user_exists,
    request_review_transactions::persist_review_mutation,
    request_rows::{credit_account_by_user_id, request_by_id},
};
use crate::{
    domain::{
        requests::{MergeRequestInput, RequestActorRole, RequestReviewMutation, merge_request},
        reviewed_updates::ReviewedUpdateInput,
        store::{GitHead, RepoPublicationState},
    },
    error::ApiError,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};

#[derive(Clone, Debug)]
pub struct MergeRequestContentMutation {
    pub request: RequestReviewMutation,
    pub git_head: GitHead,
}

impl MetadataStore {
    #[allow(clippy::too_many_arguments)]
    pub async fn merge_request_content(
        &self,
        owner: &str,
        name: &str,
        expected_manifest_key: &str,
        expected_repo_change_version: u64,
        expected_request_head_oid: &str,
        update: ReviewedUpdateInput,
        mut input: MergeRequestInput,
    ) -> Result<MergeRequestContentMutation, ApiError> {
        let repo_id = crate::domain::store::repo_id(owner, name);
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        acquire_aggregate_lock(&tx, "request", &input.request_id).await?;

        let request = request_by_id(&tx, &input.request_id)
            .await?
            .filter(|request| request.repo_id == repo_id)
            .ok_or_else(|| ApiError::not_found("request not found"))?;
        if request.head_oid != expected_request_head_oid {
            return Err(ApiError::conflict(
                "request changed since merge was prepared; retry merge",
            ));
        }
        if request.author_role == RequestActorRole::Public {
            acquire_aggregate_lock(&tx, "user-credit", &request.author_user_id).await?;
        }
        ensure_user_exists(&tx, &input.actor_user_id).await?;

        let repo_row = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
        let repo_change_version = u64::try_from(repo_row.change_version)
            .map_err(|_| ApiError::internal_message("repository change version is negative"))?;
        if repo_change_version != expected_repo_change_version {
            return Err(ApiError::conflict(
                "repo changed since merge was prepared; retry merge",
            ));
        }
        let publication_state: RepoPublicationState = serde_json::from_value(
            serde_json::Value::String(repo_row.publication_state.clone()),
        )
        .map_err(ApiError::internal)?;
        if publication_state != RepoPublicationState::Published {
            return Err(ApiError::conflict("repo must be published before merge"));
        }
        let head = entities::git_head::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::conflict("repo has no accepted Git head"))?
            .try_into_domain()?;
        if head.manifest.object_key != expected_manifest_key {
            return Err(ApiError::conflict(
                "repo changed since merge was prepared; retry merge",
            ));
        }
        let is_member = entities::repository_member::Entity::find()
            .filter(entities::repository_member::Column::RepoId.eq(repo_id.clone()))
            .filter(entities::repository_member::Column::UserId.eq(input.actor_user_id.clone()))
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .is_some();
        if repo_row.owner_user_id != input.actor_user_id && !is_member {
            return Err(ApiError::forbidden("repo maintainer required"));
        }
        input.actor_is_maintainer = true;
        input.merged_head_oid = expected_request_head_oid.to_string();
        input.merged_main_oid = update.git_head.head_oid.clone();

        let git_head = accept_and_persist_request_merge(&tx, &repo_id, repo_row, update).await?;

        let account = if request.author_role == RequestActorRole::Public
            && request.state == crate::domain::requests::RequestState::ReadyForReview
        {
            credit_account_by_user_id(&tx, &request.author_user_id).await?
        } else {
            None
        };
        let request_mutation = merge_request(&request, account.as_ref(), input)?;
        persist_review_mutation(&tx, &request_mutation).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(MergeRequestContentMutation {
            request: request_mutation,
            git_head,
        })
    }
}
