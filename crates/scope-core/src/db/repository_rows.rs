use super::{entities, outbox::enqueue_projection_read_model_rebuild};
use crate::{
    domain::store::{FirstPushToken, GitPushToken, SourceBlob, StoredRepository},
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
    pub git_snapshot: Option<SourceBlob>,
}

impl RepositoryFactRows {
    pub fn into_facts(self) -> entities::RepositoryFacts {
        entities::RepositoryFacts {
            first_push_token: self.first_push_token,
            git_push_token: self.git_push_token,
            git_snapshot: self.git_snapshot,
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
    insert_repository_relations(conn, repo).await?;
    enqueue_projection_read_model_rebuild(conn, repo).await?;
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
    set_if_changed!(graph, before_row.graph, row.graph);
    set_if_changed!(
        visibility_events,
        before_row.visibility_events,
        row.visibility_events
    );
    if row_changed {
        active.update(conn).await.map_err(ApiError::internal)?;
    }

    save_repository_fact_delta(conn, before, after).await?;
    save_repository_relation_delta(conn, before, after).await?;
    enqueue_projection_read_model_rebuild(conn, after).await?;
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
    if let Some(snapshot) = repo.git_snapshot.as_ref() {
        entities::repository_git_snapshot::Model::from_domain(&repo_id, snapshot)?
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
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
    if before.git_snapshot != after.git_snapshot {
        entities::repository_git_snapshot::Entity::delete_by_id(repo_id.clone())
            .exec(conn)
            .await
            .map_err(ApiError::internal)?;
        if let Some(snapshot) = &after.git_snapshot {
            entities::repository_git_snapshot::Model::from_domain(repo_id, snapshot)?
                .into_active_model()
                .insert(conn)
                .await
                .map_err(ApiError::internal)?;
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

    let git_snapshots = entities::repository_git_snapshot::Entity::find()
        .filter(entities::repository_git_snapshot::Column::RepoId.is_in(repo_ids.to_vec()))
        .order_by_asc(entities::repository_git_snapshot::Column::RepoId)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    for row in git_snapshots {
        if let Some(fact) = facts.get_mut(&row.repo_id) {
            fact.git_snapshot = Some(row.try_into_domain()?);
        }
    }

    Ok(facts)
}
