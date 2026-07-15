use super::{
    MetadataStore, acquire_aggregate_lock, entities,
    history_rows::{RepositoryHistory, insert_commits, save_live_file},
    object_references::{insert_object_reference, replace_object_reference},
    outbox::enqueue_projection_read_model_rebuild,
};
use crate::{
    domain::{
        projection::{AuthorVisibility, LogicalCommit, SourceGraph},
        repo_actions::reviewed_update_api_error,
        reviewed_updates::{ReviewedUpdateInput, apply_reviewed_update_to_repo},
        store::{MainPushMode, RepoPublicationState, RepositoryActor},
    },
    error::ApiError,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    TransactionTrait,
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
        if repo_row.publication_state != "Published" {
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
        let members = entities::repository_member::Entity::find()
            .filter(entities::repository_member::Column::RepoId.eq(repo_id.clone()))
            .all(&tx)
            .await
            .map_err(ApiError::internal)?
            .into_iter()
            .map(entities::repository_member::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
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
        let history = RepositoryHistory {
            graph: SourceGraph {
                repo_id: repo_id.clone(),
                commits: previous_commit
                    .map(|commit| LogicalCommit {
                        id: commit.id,
                        parent_ids: Vec::new(),
                        author_id: String::new(),
                        author_visibility: AuthorVisibility::Visible,
                        message: String::new(),
                        changes: Vec::new(),
                    })
                    .into_iter()
                    .collect(),
            },
            visibility_events: Vec::new(),
            live_files,
        };
        let mut repo = repo_row.try_into_domain(
            super::repository_rows::RepositoryFactRows {
                git_head: Some(head),
                ..Default::default()
            }
            .into_facts(),
            members,
            Vec::new(),
            history,
        )?;
        if repo.record.publication_state != RepoPublicationState::Published {
            return Ok(None);
        }
        let push_policy = repo.push_policy_for_user_id(author_id);
        if push_policy.mode != MainPushMode::Published {
            let message = if push_policy.access.actor == RepositoryActor::Public {
                "repo membership required"
            } else {
                "push permission required"
            };
            return Err(ApiError::forbidden(message));
        }
        if repo.repo_config != update.config {
            return Err(ApiError::conflict(
                "repo config changed since review; rerun scope push",
            ));
        }
        update.previous_config = Some(repo.repo_config.clone());
        update.git_head.change_version = repo.record.change_version.saturating_add(1);
        apply_reviewed_update_to_repo(&mut repo, update).map_err(reviewed_update_api_error)?;

        entities::repository::Model::from_domain(&repo)?
            .into_active_model()
            .update(&tx)
            .await
            .map_err(ApiError::internal)?;
        let new_head = repo
            .git_head
            .as_ref()
            .ok_or_else(|| ApiError::internal_message("content push did not produce a Git head"))?;
        entities::git_head::Entity::delete_by_id(repo_id.clone())
            .exec(&tx)
            .await
            .map_err(ApiError::internal)?;
        entities::git_head::Model::from_domain(&repo_id, new_head)?
            .into_active_model()
            .insert(&tx)
            .await
            .map_err(ApiError::internal)?;
        replace_object_reference(&tx, "git_manifest", &repo_id, Some(&new_head.manifest)).await?;
        let segment = repo.git_segments.last().ok_or_else(|| {
            ApiError::internal_message("content push did not produce a Git segment")
        })?;
        entities::git_segment::Model::from_domain(&repo_id, segment)?
            .into_active_model()
            .insert(&tx)
            .await
            .map_err(ApiError::internal)?;
        let segment_ref_id = format!("{repo_id}:{}", segment.sequence);
        insert_object_reference(&tx, "git_segment", &segment_ref_id, &segment.object).await?;
        insert_object_reference(
            &tx,
            "git_segment_manifest",
            &segment_ref_id,
            &segment.manifest,
        )
        .await?;
        let commit = repo.graph.commits.last().ok_or_else(|| {
            ApiError::internal_message("content push did not produce a logical commit")
        })?;
        let ordinal = usize::try_from(next_ordinal)
            .map_err(|_| ApiError::internal_message("logical commit ordinal is invalid"))?;
        insert_commits(&tx, &repo_id, ordinal, std::slice::from_ref(commit)).await?;
        for change in &commit.changes {
            save_live_file(
                &tx,
                &repo_id,
                &change.path,
                repo.live_files.get(&change.path),
            )
            .await?;
        }
        enqueue_projection_read_model_rebuild(&tx, &repo).await?;
        let committed_head = new_head.clone();
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(Some(committed_head))
    }
}
