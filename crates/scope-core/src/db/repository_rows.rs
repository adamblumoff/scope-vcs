use super::{
    entities,
    history_rows::{
        RepositoryHistoryDelta, insert_repository_history, insert_repository_live_files,
        save_repository_history_delta,
    },
    object_references::{
        delete_object_reference, insert_object_reference, replace_object_reference,
    },
    outbox::enqueue_projection_read_model_rebuild,
};
use crate::{
    domain::store::{FirstPushToken, GitHead, GitPushToken, GitSegment, StoredRepository},
    error::ApiError,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder,
};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct RepositoryFactRows {
    pub first_push_token: Option<FirstPushToken>,
    pub git_push_token: Option<GitPushToken>,
    pub git_head: Option<GitHead>,
    pub git_segments: Vec<GitSegment>,
}

impl RepositoryFactRows {
    pub fn into_facts(self) -> entities::RepositoryFacts {
        entities::RepositoryFacts {
            first_push_token: self.first_push_token,
            git_push_token: self.git_push_token,
            git_head: self.git_head,
            git_segments: self.git_segments,
        }
    }
}

pub async fn insert_repository<C>(conn: &C, repo: &StoredRepository) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::repository::Model::from_domain(repo)?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;
    insert_repository_fact_rows(conn, repo).await?;
    insert_repository_history(conn, &repo.graph, &repo.visibility_events).await?;
    insert_repository_live_files(conn, &repo.record.id, &repo.live_files).await?;
    insert_repository_relations(conn, repo).await?;
    enqueue_projection_read_model_rebuild(conn, &repo.record.id, repo.record.change_version)
        .await?;
    Ok(())
}

pub async fn save_repository_delta<C>(
    conn: &C,
    before: &StoredRepository,
    after: &StoredRepository,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    if before.record.id != after.record.id {
        return Err(ApiError::internal_message(
            "repository mutation cannot change repository identity",
        ));
    }

    let before_row = entities::repository::Model::from_domain(before)?;
    let row = entities::repository::Model::from_domain(after)?;
    let mut active = row.clone().into_active_model();
    let mut row_changed = false;
    macro_rules! set_if_changed {
        ($field:ident, $before:expr, $after:expr) => {
            if $before != $after {
                active.$field = Set(row.$field.clone());
                row_changed = true;
            }
        };
    }
    set_if_changed!(owner_handle, before_row.owner_handle, row.owner_handle);
    set_if_changed!(name, before_row.name, row.name);
    set_if_changed!(owner_user_id, before_row.owner_user_id, row.owner_user_id);
    set_if_changed!(
        publication_state,
        before_row.publication_state,
        row.publication_state
    );
    set_if_changed!(
        default_visibility,
        before_row.default_visibility,
        row.default_visibility
    );
    set_if_changed!(
        change_version,
        before_row.change_version,
        row.change_version
    );
    set_if_changed!(repo_config, before_row.repo_config, row.repo_config);
    set_if_changed!(policy, before_row.policy, row.policy);
    if row_changed {
        active.update(conn).await.map_err(ApiError::internal)?;
    }

    save_repository_fact_delta(conn, before, after).await?;
    save_repository_history_delta(
        conn,
        RepositoryHistoryDelta {
            before_graph: &before.graph,
            after_graph: &after.graph,
            before_events: &before.visibility_events,
            after_events: &after.visibility_events,
            before_live_files: &before.live_files,
            after_live_files: &after.live_files,
            history_rewritten: before.repo_config.history.rewrites
                != after.repo_config.history.rewrites,
        },
    )
    .await?;
    save_repository_relation_delta(conn, before, after).await?;
    enqueue_projection_read_model_rebuild(conn, &after.record.id, after.record.change_version)
        .await?;
    Ok(())
}

async fn insert_repository_fact_rows<C>(conn: &C, repo: &StoredRepository) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let repo_id = repo.record.id.clone();

    if let Some(token) = repo.first_push_token.as_ref() {
        entities::repository_first_push_token::Model::from_domain(&repo_id, token)?
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }
    if let Some(token) = repo.git_push_token.as_ref() {
        entities::repository_git_push_token::Model::from_domain(&repo_id, token)?
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }
    if let Some(head) = repo.git_head.as_ref() {
        entities::git_head::Model::from_domain(&repo_id, head)?
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
        insert_object_reference(conn, "git_manifest", &repo_id, &head.manifest).await?;
    }
    for segment in &repo.git_segments {
        entities::git_segment::Model::from_domain(&repo_id, segment)?
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
        insert_object_reference(
            conn,
            "git_segment",
            &format!("{repo_id}:{}", segment.sequence),
            &segment.object,
        )
        .await?;
        insert_object_reference(
            conn,
            "git_segment_manifest",
            &format!("{repo_id}:{}", segment.sequence),
            &segment.manifest,
        )
        .await?;
    }

    Ok(())
}

async fn save_repository_fact_delta<C>(
    conn: &C,
    before: &StoredRepository,
    after: &StoredRepository,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let repo_id = &after.record.id;
    if before.first_push_token != after.first_push_token {
        entities::repository_first_push_token::Entity::delete_by_id(repo_id.clone())
            .exec(conn)
            .await
            .map_err(ApiError::internal)?;
        if let Some(token) = &after.first_push_token {
            entities::repository_first_push_token::Model::from_domain(repo_id, token)?
                .into_active_model()
                .insert(conn)
                .await
                .map_err(ApiError::internal)?;
        }
    }
    if before.git_push_token != after.git_push_token {
        entities::repository_git_push_token::Entity::delete_by_id(repo_id.clone())
            .exec(conn)
            .await
            .map_err(ApiError::internal)?;
        if let Some(token) = &after.git_push_token {
            entities::repository_git_push_token::Model::from_domain(repo_id, token)?
                .into_active_model()
                .insert(conn)
                .await
                .map_err(ApiError::internal)?;
        }
    }
    if before.git_head != after.git_head {
        entities::git_head::Entity::delete_by_id(repo_id.clone())
            .exec(conn)
            .await
            .map_err(ApiError::internal)?;
        if let Some(head) = &after.git_head {
            entities::git_head::Model::from_domain(repo_id, head)?
                .into_active_model()
                .insert(conn)
                .await
                .map_err(ApiError::internal)?;
        }
        replace_object_reference(
            conn,
            "git_manifest",
            repo_id,
            after.git_head.as_ref().map(|head| &head.manifest),
        )
        .await?;
    }
    let segments_are_append_only = after.git_segments.len() >= before.git_segments.len()
        && after.git_segments[..before.git_segments.len()] == before.git_segments[..];
    if !segments_are_append_only {
        for segment in &before.git_segments {
            delete_object_reference(
                conn,
                "git_segment",
                &format!("{repo_id}:{}", segment.sequence),
            )
            .await?;
            delete_object_reference(
                conn,
                "git_segment_manifest",
                &format!("{repo_id}:{}", segment.sequence),
            )
            .await?;
        }
        entities::git_segment::Entity::delete_many()
            .filter(entities::git_segment::Column::RepoId.eq(repo_id.clone()))
            .exec(conn)
            .await
            .map_err(ApiError::internal)?;
        for segment in &after.git_segments {
            entities::git_segment::Model::from_domain(repo_id, segment)?
                .into_active_model()
                .insert(conn)
                .await
                .map_err(ApiError::internal)?;
            insert_object_reference(
                conn,
                "git_segment",
                &format!("{repo_id}:{}", segment.sequence),
                &segment.object,
            )
            .await?;
            insert_object_reference(
                conn,
                "git_segment_manifest",
                &format!("{repo_id}:{}", segment.sequence),
                &segment.manifest,
            )
            .await?;
        }
    } else {
        for segment in &after.git_segments[before.git_segments.len()..] {
            entities::git_segment::Model::from_domain(repo_id, segment)?
                .into_active_model()
                .insert(conn)
                .await
                .map_err(ApiError::internal)?;
            insert_object_reference(
                conn,
                "git_segment",
                &format!("{repo_id}:{}", segment.sequence),
                &segment.object,
            )
            .await?;
            insert_object_reference(
                conn,
                "git_segment_manifest",
                &format!("{repo_id}:{}", segment.sequence),
                &segment.manifest,
            )
            .await?;
        }
    }
    Ok(())
}

async fn insert_repository_relations<C>(conn: &C, repo: &StoredRepository) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    for member in &repo.members {
        entities::repository_member::Model::from_domain(member)?
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }

    for invite in &repo.invitations {
        entities::repository_invite::Model::from_domain(invite)?
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }

    Ok(())
}

async fn save_repository_relation_delta<C>(
    conn: &C,
    before: &StoredRepository,
    after: &StoredRepository,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let before_members = before
        .members
        .iter()
        .map(|member| (member.user_id.as_str(), member))
        .collect::<BTreeMap<_, _>>();
    let after_members = after
        .members
        .iter()
        .map(|member| (member.user_id.as_str(), member))
        .collect::<BTreeMap<_, _>>();
    for user_id in before_members.keys() {
        if !after_members.contains_key(user_id) {
            entities::repository_member::Entity::delete_by_id((
                after.record.id.clone(),
                (*user_id).to_string(),
            ))
            .exec(conn)
            .await
            .map_err(ApiError::internal)?;
        }
    }
    for (user_id, member) in after_members {
        if before_members
            .get(user_id)
            .is_some_and(|old| *old == member)
        {
            continue;
        }
        entities::repository_member::Entity::delete_by_id((
            after.record.id.clone(),
            user_id.to_string(),
        ))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
        entities::repository_member::Model::from_domain(member)?
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }

    let before_invites = before
        .invitations
        .iter()
        .map(|invite| (invite.id.as_str(), invite))
        .collect::<BTreeMap<_, _>>();
    let after_invites = after
        .invitations
        .iter()
        .map(|invite| (invite.id.as_str(), invite))
        .collect::<BTreeMap<_, _>>();
    for invite_id in before_invites.keys() {
        if !after_invites.contains_key(invite_id) {
            entities::repository_invite::Entity::delete_by_id((*invite_id).to_string())
                .exec(conn)
                .await
                .map_err(ApiError::internal)?;
        }
    }
    for (invite_id, invite) in after_invites {
        if before_invites
            .get(invite_id)
            .is_some_and(|old| *old == invite)
        {
            continue;
        }
        entities::repository_invite::Entity::delete_by_id(invite_id.to_string())
            .exec(conn)
            .await
            .map_err(ApiError::internal)?;
        entities::repository_invite::Model::from_domain(invite)?
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }
    Ok(())
}

pub async fn load_repository_facts<C>(
    conn: &C,
    repo_ids: &[String],
) -> Result<BTreeMap<String, RepositoryFactRows>, ApiError>
where
    C: ConnectionTrait,
{
    let mut facts = repo_ids
        .iter()
        .map(|repo_id| (repo_id.clone(), RepositoryFactRows::default()))
        .collect::<BTreeMap<_, _>>();
    if repo_ids.is_empty() {
        return Ok(facts);
    }

    let first_push_tokens = entities::repository_first_push_token::Entity::find()
        .filter(entities::repository_first_push_token::Column::RepoId.is_in(repo_ids.to_vec()))
        .order_by_asc(entities::repository_first_push_token::Column::RepoId)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    for row in first_push_tokens {
        if let Some(fact) = facts.get_mut(&row.repo_id) {
            fact.first_push_token = Some(row.try_into_domain()?);
        }
    }

    let git_push_tokens = entities::repository_git_push_token::Entity::find()
        .filter(entities::repository_git_push_token::Column::RepoId.is_in(repo_ids.to_vec()))
        .order_by_asc(entities::repository_git_push_token::Column::RepoId)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    for row in git_push_tokens {
        if let Some(fact) = facts.get_mut(&row.repo_id) {
            fact.git_push_token = Some(row.try_into_domain()?);
        }
    }

    let git_heads = entities::git_head::Entity::find()
        .filter(entities::git_head::Column::RepoId.is_in(repo_ids.to_vec()))
        .order_by_asc(entities::git_head::Column::RepoId)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    for row in git_heads {
        if let Some(fact) = facts.get_mut(&row.repo_id) {
            fact.git_head = Some(row.try_into_domain()?);
        }
    }
    let git_segments = entities::git_segment::Entity::find()
        .filter(entities::git_segment::Column::RepoId.is_in(repo_ids.to_vec()))
        .order_by_asc(entities::git_segment::Column::RepoId)
        .order_by_asc(entities::git_segment::Column::Sequence)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    for row in git_segments {
        if let Some(fact) = facts.get_mut(&row.repo_id) {
            fact.git_segments.push(row.try_into_domain()?);
        }
    }

    Ok(facts)
}
