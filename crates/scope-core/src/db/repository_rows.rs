use super::{entities, outbox::enqueue_projection_read_model_rebuild};
use crate::{
    domain::store::{FirstPushToken, GitPushToken, RepoSettings, SourceBlob, StoredRepository},
    error::ApiError,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, sea_query::Expr,
};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct RepositoryFactRows {
    pub settings: Option<RepoSettings>,
    pub first_push_token: Option<FirstPushToken>,
    pub git_push_token: Option<GitPushToken>,
    pub git_snapshot: Option<SourceBlob>,
}

impl RepositoryFactRows {
    pub fn into_required(self, repo_id: &str) -> Result<entities::RepositoryFacts, ApiError> {
        let settings = self.settings.ok_or_else(|| {
            ApiError::internal_message(format!("repository settings missing for {repo_id}"))
        })?;
        Ok(entities::RepositoryFacts {
            settings,
            first_push_token: self.first_push_token,
            git_push_token: self.git_push_token,
            git_snapshot: self.git_snapshot,
        })
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
    save_repository_fact_rows(conn, repo).await?;
    save_repository_relations(conn, repo).await?;
    enqueue_projection_read_model_rebuild(conn, repo).await?;
    Ok(())
}

pub async fn save_repository_row<C>(conn: &C, repo: &StoredRepository) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let row = entities::repository::Model::from_domain(repo)?;
    entities::repository::Entity::update_many()
        .filter(entities::repository::Column::Id.eq(row.id))
        .col_expr(
            entities::repository::Column::OwnerHandle,
            Expr::value(row.owner_handle),
        )
        .col_expr(entities::repository::Column::Name, Expr::value(row.name))
        .col_expr(
            entities::repository::Column::OwnerUserId,
            Expr::value(row.owner_user_id),
        )
        .col_expr(
            entities::repository::Column::PublicationState,
            Expr::value(row.publication_state),
        )
        .col_expr(
            entities::repository::Column::DefaultVisibility,
            Expr::value(row.default_visibility),
        )
        .col_expr(
            entities::repository::Column::ChangeVersion,
            Expr::value(row.change_version),
        )
        .col_expr(
            entities::repository::Column::RepoConfig,
            Expr::value(row.repo_config),
        )
        .col_expr(
            entities::repository::Column::PendingImport,
            Expr::value(row.pending_import),
        )
        .col_expr(
            entities::repository::Column::Policy,
            Expr::value(row.policy),
        )
        .col_expr(entities::repository::Column::Graph, Expr::value(row.graph))
        .col_expr(
            entities::repository::Column::VisibilityEvents,
            Expr::value(row.visibility_events),
        )
        .col_expr(
            entities::repository::Column::StagedUpdate,
            Expr::value(row.staged_update),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    save_repository_fact_rows(conn, repo).await?;
    save_repository_relations(conn, repo).await?;
    enqueue_projection_read_model_rebuild(conn, repo).await?;
    Ok(())
}

pub async fn save_repository_fact_rows<C>(conn: &C, repo: &StoredRepository) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let repo_id = repo.record.id.clone();
    delete_repository_fact_rows(conn, &repo_id).await?;

    entities::repository_setting::Model::from_domain(&repo_id, repo.settings)
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;

    if let Some(token) = repo.first_push_token.as_ref() {
        entities::repository_first_push_token::Model::from_domain(&repo_id, token)
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }
    if let Some(token) = repo.git_push_token.as_ref() {
        entities::repository_git_push_token::Model::from_domain(&repo_id, token)
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }
    if let Some(snapshot) = repo.git_snapshot.as_ref() {
        entities::repository_git_snapshot::Model::from_domain(&repo_id, snapshot)
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }

    Ok(())
}

async fn delete_repository_fact_rows<C>(conn: &C, repo_id: &str) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::repository_setting::Entity::delete_many()
        .filter(entities::repository_setting::Column::RepoId.eq(repo_id.to_string()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    entities::repository_first_push_token::Entity::delete_many()
        .filter(entities::repository_first_push_token::Column::RepoId.eq(repo_id.to_string()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    entities::repository_git_push_token::Entity::delete_many()
        .filter(entities::repository_git_push_token::Column::RepoId.eq(repo_id.to_string()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    entities::repository_git_snapshot::Entity::delete_many()
        .filter(entities::repository_git_snapshot::Column::RepoId.eq(repo_id.to_string()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

pub async fn save_repository_relations<C>(conn: &C, repo: &StoredRepository) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::repository_member::Entity::delete_many()
        .filter(entities::repository_member::Column::RepoId.eq(repo.record.id.clone()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    for member in &repo.members {
        entities::repository_member::Model::from_domain(member)?
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }

    entities::repository_invite::Entity::delete_many()
        .filter(entities::repository_invite::Column::RepoId.eq(repo.record.id.clone()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    for invite in &repo.invitations {
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

    let settings = entities::repository_setting::Entity::find()
        .filter(entities::repository_setting::Column::RepoId.is_in(repo_ids.to_vec()))
        .order_by_asc(entities::repository_setting::Column::RepoId)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    for row in settings {
        if let Some(fact) = facts.get_mut(&row.repo_id) {
            fact.settings = Some(row.into_domain());
        }
    }

    let first_push_tokens = entities::repository_first_push_token::Entity::find()
        .filter(entities::repository_first_push_token::Column::RepoId.is_in(repo_ids.to_vec()))
        .order_by_asc(entities::repository_first_push_token::Column::RepoId)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    for row in first_push_tokens {
        if let Some(fact) = facts.get_mut(&row.repo_id) {
            fact.first_push_token = Some(row.into_domain());
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
            fact.git_push_token = Some(row.into_domain());
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
            fact.git_snapshot = Some(row.into_domain());
        }
    }

    Ok(facts)
}
