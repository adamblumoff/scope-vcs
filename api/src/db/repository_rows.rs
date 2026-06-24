use super::entities;
use crate::{domain::store::StoredRepository, error::ApiError};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, sea_query::Expr};

pub(super) async fn save_repository_row<C>(
    conn: &C,
    repo: &StoredRepository,
) -> Result<(), ApiError>
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
            entities::repository::Column::Settings,
            Expr::value(row.settings),
        )
        .col_expr(
            entities::repository::Column::FirstPushToken,
            Expr::value(row.first_push_token),
        )
        .col_expr(
            entities::repository::Column::GitPushToken,
            Expr::value(row.git_push_token),
        )
        .col_expr(
            entities::repository::Column::GitCloneTokens,
            Expr::value(row.git_clone_tokens),
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
            entities::repository::Column::GitSnapshot,
            Expr::value(row.git_snapshot),
        )
        .col_expr(
            entities::repository::Column::StagedUpdate,
            Expr::value(row.staged_update),
        )
        .col_expr(
            entities::repository::Column::Invitations,
            Expr::value(row.invitations),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}
