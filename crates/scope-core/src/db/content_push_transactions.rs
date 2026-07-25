//! Shared repository-content persistence for transactions with additional domain effects.

use super::{
    entities,
    history_rows::{insert_commits, save_live_file},
    object_references::{insert_object_reference, replace_object_reference},
    outbox::enqueue_projection_read_model_rebuild,
};
use crate::{
    domain::{
        policy::{Policy, ScopePath},
        repo_actions::reviewed_update_api_error,
        repo_config::RepoConfig,
        reviewed_updates::{
            AcceptedContentPush, ContentPushState, ReviewedUpdateInput, accept_content_push,
            accept_request_merge,
        },
        store::GitHead,
    },
    error::ApiError,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder,
};
use std::collections::BTreeMap;

pub(super) async fn accept_and_persist_content_push(
    tx: &DatabaseTransaction,
    repo_id: &str,
    repo_row: entities::repository::Model,
    update: ReviewedUpdateInput,
) -> Result<GitHead, ApiError> {
    accept_and_persist_content_update(tx, repo_id, repo_row, update, false).await
}

pub(super) async fn accept_and_persist_request_merge(
    tx: &DatabaseTransaction,
    repo_id: &str,
    repo_row: entities::repository::Model,
    update: ReviewedUpdateInput,
) -> Result<GitHead, ApiError> {
    accept_and_persist_content_update(tx, repo_id, repo_row, update, true).await
}

async fn accept_and_persist_content_update(
    tx: &DatabaseTransaction,
    repo_id: &str,
    repo_row: entities::repository::Model,
    mut update: ReviewedUpdateInput,
    request_merge: bool,
) -> Result<GitHead, ApiError> {
    let changed_paths = update
        .changes
        .iter()
        .map(|change| change.path.as_str().to_string())
        .collect::<Vec<_>>();
    let live_files = entities::live_file::Entity::find()
        .filter(entities::live_file::Column::RepoId.eq(repo_id))
        .filter(entities::live_file::Column::Path.is_in(changed_paths))
        .all(tx)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|row| {
            Ok((
                ScopePath::parse(row.path).map_err(ApiError::internal)?,
                serde_json::from_value(row.content).map_err(ApiError::internal)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, ApiError>>()?;
    let previous_commit = entities::logical_commit::Entity::find()
        .filter(entities::logical_commit::Column::RepoId.eq(repo_id))
        .order_by_desc(entities::logical_commit::Column::Ordinal)
        .one(tx)
        .await
        .map_err(ApiError::internal)?;
    let next_ordinal = previous_commit
        .as_ref()
        .map_or(0, |commit| commit.ordinal.saturating_add(1));
    let repo_config: RepoConfig =
        serde_json::from_value(repo_row.repo_config.clone()).map_err(ApiError::internal)?;
    let policy: Policy =
        serde_json::from_value(repo_row.policy.clone()).map_err(ApiError::internal)?;
    let change_version = u64::try_from(repo_row.change_version)
        .map_err(|_| ApiError::internal_message("repository change version cannot be negative"))?;
    update.previous_config = Some(repo_config.clone());
    let AcceptedContentPush {
        change_version,
        policy,
        git_head,
        git_segment,
        logical_commit,
    } = {
        let state = ContentPushState {
            change_version,
            policy,
            repo_config,
            previous_commit_id: previous_commit.map(|commit| commit.id),
            live_files,
        };
        if request_merge {
            accept_request_merge(state, update)
        } else {
            accept_content_push(state, update)
        }
        .map_err(reviewed_update_api_error)?
    };

    let persisted_change_version = i64::try_from(change_version).map_err(|_| {
        ApiError::internal_message("repository change version exceeds PostgreSQL bigint range")
    })?;
    let mut repo_update = repo_row.into_active_model();
    repo_update.change_version = Set(persisted_change_version);
    repo_update.policy = Set(serde_json::to_value(&policy).map_err(ApiError::internal)?);
    repo_update.update(tx).await.map_err(ApiError::internal)?;
    entities::git_head::Entity::delete_by_id(repo_id)
        .exec(tx)
        .await
        .map_err(ApiError::internal)?;
    entities::git_head::Model::from_domain(repo_id, &git_head)?
        .into_active_model()
        .insert(tx)
        .await
        .map_err(ApiError::internal)?;
    replace_object_reference(tx, "git_manifest", repo_id, Some(&git_head.manifest)).await?;
    entities::git_segment::Model::from_domain(repo_id, &git_segment)?
        .into_active_model()
        .insert(tx)
        .await
        .map_err(ApiError::internal)?;
    let segment_ref_id = format!("{repo_id}:{}", git_segment.sequence);
    insert_object_reference(tx, "git_segment", &segment_ref_id, &git_segment.object).await?;
    insert_object_reference(
        tx,
        "git_segment_manifest",
        &segment_ref_id,
        &git_segment.manifest,
    )
    .await?;
    let ordinal = usize::try_from(next_ordinal)
        .map_err(|_| ApiError::internal_message("logical commit ordinal is invalid"))?;
    insert_commits(tx, repo_id, ordinal, std::slice::from_ref(&logical_commit)).await?;
    for change in &logical_commit.changes {
        save_live_file(tx, repo_id, &change.path, change.new_content.as_ref()).await?;
    }
    enqueue_projection_read_model_rebuild(tx, repo_id, change_version).await?;
    Ok(git_head)
}
