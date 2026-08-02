//! Atomic repository content merge plus request completion.

use super::{
    GeneratedIdSource, RequestStore, acquire_aggregate_lock,
    content_push_transactions::accept_and_persist_request_merge, entities,
    request_access::ensure_user_exists, request_rows::request_by_id,
    request_submission_transactions::persist_lifecycle_mutation,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use {
    crate::error::PostgresError,
    scope_domain::{
        requests::{MergeRequestInput, RequestLifecycleMutation, merge_request},
        reviewed_updates::ReviewedUpdateInput,
        store::{GitHead, RepoPublicationState, RequestMergeOrigin},
    },
};

#[derive(Clone, Debug)]
pub struct MergeRequestContentMutation {
    pub request: RequestLifecycleMutation,
    pub git_head: GitHead,
}

impl RequestStore {
    #[allow(clippy::too_many_arguments)]
    pub async fn merge_request_content(
        &self,
        owner: &str,
        name: &str,
        expected_manifest_ref: &scope_domain::content_ref::ContentRef,
        expected_repo_change_version: u64,
        expected_request_head_oid: &str,
        update: ReviewedUpdateInput,
        origin: RequestMergeOrigin,
        mut input: MergeRequestInput,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<MergeRequestContentMutation, PostgresError> {
        let now_unix = input.now_unix;
        let repo_id = scope_domain::store::repo_id(owner, name);
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        acquire_aggregate_lock(&tx, "request", &input.request_id).await?;

        let request = request_by_id(&tx, &input.request_id)
            .await?
            .filter(|request| request.repo_id == repo_id)
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        if request.head_oid != expected_request_head_oid {
            return Err(PostgresError::conflict(
                "request changed since merge was prepared; retry merge",
            ));
        }
        ensure_user_exists(&tx, &input.actor_user_id).await?;

        let repo_row = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
        let repo_change_version = u64::try_from(repo_row.change_version).map_err(|_| {
            PostgresError::internal_message("repository change version is negative")
        })?;
        if repo_change_version != expected_repo_change_version {
            return Err(PostgresError::conflict(
                "repo changed since merge was prepared; retry merge",
            ));
        }
        let publication_state: RepoPublicationState = serde_json::from_value(
            serde_json::Value::String(repo_row.publication_state.clone()),
        )
        .map_err(PostgresError::internal)?;
        if publication_state != RepoPublicationState::Published {
            return Err(PostgresError::conflict(
                "repo must be published before merge",
            ));
        }
        let head = entities::git_head::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::conflict("repo has no accepted Git head"))?
            .try_into_domain()?;
        if &head.manifest.content_ref != expected_manifest_ref {
            return Err(PostgresError::conflict(
                "repo changed since merge was prepared; retry merge",
            ));
        }
        let is_member = entities::repository_member::Entity::find()
            .filter(entities::repository_member::Column::RepoId.eq(repo_id.clone()))
            .filter(entities::repository_member::Column::UserId.eq(input.actor_user_id.clone()))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .is_some();
        if repo_row.owner_user_id != input.actor_user_id && !is_member {
            return Err(PostgresError::permission_denied("repo maintainer required"));
        }
        input.actor_is_maintainer = true;
        input.merged_head_oid = expected_request_head_oid.to_string();
        input.merged_main_oid = update.git_head.head_oid.clone();

        let git_head = accept_and_persist_request_merge(
            &tx,
            &repo_id,
            repo_row,
            update,
            origin,
            now_unix,
            generated_ids,
        )
        .await?;

        let request_mutation = merge_request(&request, input)?;
        persist_lifecycle_mutation(&tx, &request_mutation).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(MergeRequestContentMutation {
            request: request_mutation,
            git_head,
        })
    }
}
