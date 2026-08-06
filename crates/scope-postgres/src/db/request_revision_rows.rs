use super::{entities, object_references::replace_object_reference};
use crate::error::PostgresError;
use scope_domain::requests::RequestRevision;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect,
};

#[derive(Clone, Debug)]
pub struct RequestRevisionWindow {
    pub revisions: Vec<RequestRevision>,
    pub has_earlier_revisions: bool,
}

pub async fn revision_by_id<C>(conn: &C, id: &str) -> Result<Option<RequestRevision>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request_revision::Entity::find_by_id(id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .map(entities::request_revision::Model::try_into_domain)
        .transpose()
}

pub async fn revisions_for_request_ids<C>(
    conn: &C,
    request_ids: &[String],
) -> Result<Vec<RequestRevision>, PostgresError>
where
    C: ConnectionTrait,
{
    if request_ids.is_empty() {
        return Ok(Vec::new());
    }
    entities::request_revision::Entity::find()
        .filter(entities::request_revision::Column::RequestId.is_in(request_ids.iter().cloned()))
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::request_revision::Model::try_into_domain)
        .collect()
}

pub async fn revision_window_for_request<C>(
    conn: &C,
    request_id: &str,
    selected_revision_id: Option<&str>,
    limit: u64,
) -> Result<RequestRevisionWindow, PostgresError>
where
    C: ConnectionTrait,
{
    let fetch_limit = limit.saturating_add(1);
    let mut models = entities::request_revision::Entity::find()
        .filter(entities::request_revision::Column::RequestId.eq(request_id))
        .order_by_desc(entities::request_revision::Column::Position)
        .order_by_desc(entities::request_revision::Column::Id)
        .limit(fetch_limit)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    let has_earlier_revisions = models.len() > limit as usize;
    models.truncate(limit as usize);
    let mut revisions = models
        .into_iter()
        .map(entities::request_revision::Model::try_into_domain)
        .collect::<Result<Vec<_>, _>>()?;

    if let Some(selected_revision_id) = selected_revision_id
        && !revisions
            .iter()
            .any(|revision| revision.id == selected_revision_id)
        && let Some(selected) = revision_by_id(conn, selected_revision_id)
            .await?
            .filter(|revision| revision.request_id == request_id)
    {
        revisions.push(selected);
    }
    revisions.sort_by(|left, right| {
        left.position
            .cmp(&right.position)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(RequestRevisionWindow {
        revisions,
        has_earlier_revisions,
    })
}

pub async fn insert_revision<C>(conn: &C, value: &RequestRevision) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    entities::request_revision::Model::from_domain(value)?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(PostgresError::internal)?;
    replace_object_reference(
        conn,
        "request_revision_snapshot",
        &value.id,
        Some(&value.git_snapshot),
    )
    .await?;
    Ok(())
}
