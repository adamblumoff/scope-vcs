use super::{
    MetadataStore, acquire_aggregate_lock, entities,
    history_rows::{insert_commits, save_live_file},
    object_references::{insert_object_reference, replace_object_reference},
    outbox::enqueue_projection_read_model_rebuild,
};
use crate::{
    domain::{
        policy::Policy,
        repo_actions::reviewed_update_api_error,
        repo_config::RepoConfig,
        reviewed_updates::{
            AcceptedContentPush, ContentPushState, ReviewedUpdateInput, accept_content_push,
        },
        store::{
            MainPushMode, RepoPublicationState, RepositoryActor, repository_push_policy_for_user_id,
        },
    },
    error::ApiError,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, TransactionTrait,
};
use std::collections::BTreeMap;

impl MetadataStore {
    pub async fn apply_content_only_push(
        &self,
        owner: &str,
        name: &str,
        author_id: &str,
        expected_manifest_key: &str,
        mut update: ReviewedUpdateInput,
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
        let changed_paths = update
            .changes
            .iter()
            .map(|change| change.path.as_str().to_string())
            .collect::<Vec<_>>();
        let live_files = entities::live_file::Entity::find()
            .filter(entities::live_file::Column::RepoId.eq(repo_id.clone()))
            .filter(entities::live_file::Column::Path.is_in(changed_paths))
            .all(&tx)
            .await
            .map_err(ApiError::internal)?
            .into_iter()
            .map(|row| {
                Ok((
                    crate::domain::policy::ScopePath::parse(row.path)
                        .map_err(ApiError::internal)?,
                    serde_json::from_value(row.content).map_err(ApiError::internal)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, ApiError>>()?;
        let previous_commit = entities::logical_commit::Entity::find()
            .filter(entities::logical_commit::Column::RepoId.eq(repo_id.clone()))
            .order_by_desc(entities::logical_commit::Column::Ordinal)
            .one(&tx)
            .await
            .map_err(ApiError::internal)?;
        let next_ordinal = previous_commit
            .as_ref()
            .map_or(0, |commit| commit.ordinal.saturating_add(1));
        let repo_config: RepoConfig =
            serde_json::from_value(repo_row.repo_config.clone()).map_err(ApiError::internal)?;
        let policy: Policy =
            serde_json::from_value(repo_row.policy.clone()).map_err(ApiError::internal)?;
        let change_version = u64::try_from(repo_row.change_version).map_err(|_| {
            ApiError::internal_message("repository change version cannot be negative")
        })?;
        update.previous_config = Some(repo_config.clone());
        let accepted = accept_content_push(
            ContentPushState {
                change_version,
                policy,
                repo_config,
                previous_commit_id: previous_commit.map(|commit| commit.id),
                live_files,
            },
            update,
        )
        .map_err(reviewed_update_api_error)?;
        let AcceptedContentPush {
            change_version,
            policy,
            git_head,
            git_segment,
            logical_commit,
        } = accepted;

        let persisted_change_version = i64::try_from(change_version).map_err(|_| {
            ApiError::internal_message("repository change version exceeds PostgreSQL bigint range")
        })?;
        let mut repo_update = repo_row.into_active_model();
        repo_update.change_version = Set(persisted_change_version);
        repo_update.policy = Set(serde_json::to_value(&policy).map_err(ApiError::internal)?);
        repo_update.update(&tx).await.map_err(ApiError::internal)?;
        entities::git_head::Entity::delete_by_id(repo_id.clone())
            .exec(&tx)
            .await
            .map_err(ApiError::internal)?;
        entities::git_head::Model::from_domain(&repo_id, &git_head)?
            .into_active_model()
            .insert(&tx)
            .await
            .map_err(ApiError::internal)?;
        replace_object_reference(&tx, "git_manifest", &repo_id, Some(&git_head.manifest)).await?;
        entities::git_segment::Model::from_domain(&repo_id, &git_segment)?
            .into_active_model()
            .insert(&tx)
            .await
            .map_err(ApiError::internal)?;
        let segment_ref_id = format!("{repo_id}:{}", git_segment.sequence);
        insert_object_reference(&tx, "git_segment", &segment_ref_id, &git_segment.object).await?;
        insert_object_reference(
            &tx,
            "git_segment_manifest",
            &segment_ref_id,
            &git_segment.manifest,
        )
        .await?;
        let ordinal = usize::try_from(next_ordinal)
            .map_err(|_| ApiError::internal_message("logical commit ordinal is invalid"))?;
        insert_commits(
            &tx,
            &repo_id,
            ordinal,
            std::slice::from_ref(&logical_commit),
        )
        .await?;
        for change in &logical_commit.changes {
            save_live_file(&tx, &repo_id, &change.path, change.new_content.as_ref()).await?;
        }
        enqueue_projection_read_model_rebuild(&tx, &repo_id, change_version).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(Some(git_head))
    }
}
