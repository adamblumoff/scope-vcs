//! Shared repository-content persistence for transactions with additional domain effects.

use super::{
    GeneratedIdSource, entities,
    git_compaction::schedule_git_compaction,
    history_rows::{insert_commits, save_live_file},
    landing_files::apply_repository_landing_file_mutation,
    object_references::{insert_object_reference, replace_object_reference},
    outbox::enqueue_projection_read_model_rebuild,
    push_triggers::enqueue_push_main_trigger_evaluation,
    workflow_catalogs::apply_repository_workflow_catalog,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder,
};
use std::collections::BTreeMap;
use {
    crate::error::PostgresError,
    scope_domain::{
        landing_file::RepositoryLandingFileMutation,
        policy::{Policy, ScopePath},
        repo_actions::reviewed_update_domain_error,
        repo_config::RepoConfig,
        repo_control::REPO_RULES_PATH,
        reviewed_updates::{
            AcceptedContentPush, ContentPushState, ReviewedUpdateInput, accept_content_push,
            accept_request_merge,
        },
        runs::catalog::RepositoryWorkflowCatalog,
        runs::trigger::PushTriggerInput,
        store::{GitHead, RequestMergeOrigin},
    },
};

pub(super) async fn accept_and_persist_content_push(
    tx: &DatabaseTransaction,
    repo_row: entities::repository::Model,
    update: ReviewedUpdateInput,
    snapshots: RepositoryContentSnapshots,
    push_trigger_input: PushTriggerInput,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<GitHead, PostgresError> {
    accept_and_persist_content_update(
        tx,
        repo_row,
        update,
        snapshots,
        ContentUpdateKind::MainPush(push_trigger_input),
        now_unix,
        generated_ids,
    )
    .await
}

pub(super) async fn accept_and_persist_request_merge(
    tx: &DatabaseTransaction,
    repo_row: entities::repository::Model,
    update: ReviewedUpdateInput,
    snapshots: RepositoryContentSnapshots,
    origin: RequestMergeOrigin,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<GitHead, PostgresError> {
    accept_and_persist_content_update(
        tx,
        repo_row,
        update,
        snapshots,
        ContentUpdateKind::RequestMerge(origin),
        now_unix,
        generated_ids,
    )
    .await
}

enum ContentUpdateKind {
    MainPush(PushTriggerInput),
    RequestMerge(RequestMergeOrigin),
}

pub(super) struct RepositoryContentSnapshots {
    pub(super) landing_file_mutation: RepositoryLandingFileMutation,
    pub(super) workflow_catalog: RepositoryWorkflowCatalog,
}

async fn accept_and_persist_content_update(
    tx: &DatabaseTransaction,
    repo_row: entities::repository::Model,
    mut update: ReviewedUpdateInput,
    snapshots: RepositoryContentSnapshots,
    kind: ContentUpdateKind,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<GitHead, PostgresError> {
    let RepositoryContentSnapshots {
        landing_file_mutation,
        workflow_catalog,
    } = snapshots;
    let repo_id = repo_row.id.clone();
    let mut changed_paths = update
        .changes
        .iter()
        .map(|change| change.path.as_str().to_string())
        .collect::<Vec<_>>();
    if !changed_paths.iter().any(|path| path == REPO_RULES_PATH) {
        changed_paths.push(REPO_RULES_PATH.to_string());
    }
    let live_files = entities::live_file::Entity::find()
        .filter(entities::live_file::Column::RepoId.eq(&repo_id))
        .filter(entities::live_file::Column::Path.is_in(changed_paths))
        .all(tx)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| {
            Ok((
                ScopePath::parse(row.path).map_err(PostgresError::internal)?,
                serde_json::from_value(row.content).map_err(PostgresError::internal)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, PostgresError>>()?;
    let previous_commit = entities::logical_commit::Entity::find()
        .filter(entities::logical_commit::Column::RepoId.eq(&repo_id))
        .order_by_desc(entities::logical_commit::Column::Ordinal)
        .one(tx)
        .await
        .map_err(PostgresError::internal)?;
    let next_ordinal = previous_commit
        .as_ref()
        .map_or(0, |commit| commit.ordinal.saturating_add(1));
    let repo_config: RepoConfig =
        serde_json::from_value(repo_row.repo_config.clone()).map_err(PostgresError::internal)?;
    let policy: Policy =
        serde_json::from_value(repo_row.policy.clone()).map_err(PostgresError::internal)?;
    let change_version = u64::try_from(repo_row.change_version).map_err(|_| {
        PostgresError::internal_message("repository change version cannot be negative")
    })?;
    let git_head = entities::git_head::Entity::find_by_id(&repo_id)
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .map(entities::git_head::Model::try_into_domain)
        .transpose()?;
    update.previous_config = Some(repo_config.clone());
    let (accepted, push_trigger_input) = {
        let state = ContentPushState {
            change_version,
            policy,
            repo_config,
            live_files,
            git_head,
        };
        match kind {
            ContentUpdateKind::MainPush(input) => (accept_content_push(state, update), Some(input)),
            ContentUpdateKind::RequestMerge(origin) => {
                (accept_request_merge(state, update, origin), None)
            }
        }
    };
    let AcceptedContentPush {
        change_version,
        policy,
        git_head,
        git_pack_span,
        logical_commit,
    } = accepted.map_err(reviewed_update_domain_error)?;
    workflow_catalog
        .verify_source(&repo_id, &git_head.head_oid, change_version)
        .map_err(PostgresError::internal)?;

    let persisted_change_version = i64::try_from(change_version).map_err(|_| {
        PostgresError::internal_message("repository change version exceeds PostgreSQL bigint range")
    })?;
    let mut repo_update = repo_row.into_active_model();
    repo_update.change_version = Set(persisted_change_version);
    repo_update.policy = Set(serde_json::to_value(&policy).map_err(PostgresError::internal)?);
    repo_update
        .update(tx)
        .await
        .map_err(PostgresError::internal)?;
    entities::git_head::Entity::delete_by_id(&repo_id)
        .exec(tx)
        .await
        .map_err(PostgresError::internal)?;
    entities::git_head::Model::from_domain(&repo_id, &git_head)?
        .into_active_model()
        .insert(tx)
        .await
        .map_err(PostgresError::internal)?;
    replace_object_reference(tx, "git_manifest", &repo_id, Some(&git_head.manifest)).await?;
    entities::git_pack_span::Model::from_domain(&repo_id, &git_pack_span)?
        .into_active_model()
        .insert(tx)
        .await
        .map_err(PostgresError::internal)?;
    let segment_ref_id = format!("{repo_id}:{}", git_pack_span.first_sequence);
    insert_object_reference(tx, "git_segment", &segment_ref_id, &git_pack_span.object).await?;
    schedule_git_compaction(tx, &repo_id, git_head.push_sequence, now_unix).await?;
    let pinned_pack_spans = entities::git_pack_span::Entity::find()
        .filter(entities::git_pack_span::Column::RepoId.eq(&repo_id))
        .order_by_asc(entities::git_pack_span::Column::FirstSequence)
        .all(tx)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::git_pack_span::Model::try_into_domain)
        .collect::<Result<Vec<_>, _>>()?;
    let ordinal = usize::try_from(next_ordinal)
        .map_err(|_| PostgresError::internal_message("logical commit ordinal is invalid"))?;
    insert_commits(tx, &repo_id, ordinal, std::slice::from_ref(&logical_commit)).await?;
    for change in &logical_commit.changes {
        save_live_file(tx, &repo_id, &change.path, change.new_content.as_ref()).await?;
    }
    apply_repository_landing_file_mutation(tx, &repo_id, landing_file_mutation).await?;
    apply_repository_workflow_catalog(tx, &workflow_catalog).await?;
    enqueue_projection_read_model_rebuild(tx, &repo_id, change_version, now_unix, generated_ids)
        .await?;
    if let Some(input) = push_trigger_input {
        enqueue_push_main_trigger_evaluation(
            tx,
            &repo_id,
            &git_head,
            &pinned_pack_spans,
            &input,
            now_unix,
            generated_ids,
        )
        .await?;
    }
    Ok(git_head)
}
