use super::{entities, object_references::replace_object_reference};
use crate::{domain::requests::RequestChangeBlock, error::ApiError};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
};
use std::collections::BTreeMap;

pub async fn change_block_by_id<C>(
    conn: &C,
    id: &str,
) -> Result<Option<RequestChangeBlock>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request_change_block::Entity::find_by_id(id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::request_change_block::Model::try_into_domain)
        .transpose()
}

pub async fn change_blocks_by_ids<C>(
    conn: &C,
    ids: &[String],
) -> Result<BTreeMap<String, RequestChangeBlock>, ApiError>
where
    C: ConnectionTrait,
{
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    entities::request_change_block::Entity::find()
        .filter(entities::request_change_block::Column::Id.is_in(ids.iter().cloned()))
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|row| {
            let block = row.try_into_domain()?;
            Ok((block.id.clone(), block))
        })
        .collect()
}

pub async fn change_blocks_for_request_ids<C>(
    conn: &C,
    request_ids: &[String],
) -> Result<Vec<RequestChangeBlock>, ApiError>
where
    C: ConnectionTrait,
{
    if request_ids.is_empty() {
        return Ok(Vec::new());
    }
    entities::request_change_block::Entity::find()
        .filter(
            entities::request_change_block::Column::RequestId.is_in(request_ids.iter().cloned()),
        )
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(entities::request_change_block::Model::try_into_domain)
        .collect()
}

pub async fn insert_change_block<C>(conn: &C, value: &RequestChangeBlock) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::request_change_block::Model::from_domain(value)?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;
    replace_object_reference(
        conn,
        "request_change_block_snapshot",
        &value.id,
        Some(&value.git_snapshot),
    )
    .await?;
    Ok(())
}
