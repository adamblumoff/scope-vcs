use super::entities;
use crate::{domain::store::SourceBlob, error::ApiError};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
};

pub async fn replace_object_reference<C>(
    conn: &C,
    ref_kind: &str,
    ref_id: &str,
    object: Option<&SourceBlob>,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::object_reference::Entity::delete_many()
        .filter(entities::object_reference::Column::RefKind.eq(ref_kind.to_string()))
        .filter(entities::object_reference::Column::RefId.eq(ref_id.to_string()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    if let Some(object) = object {
        insert_object_reference(conn, ref_kind, ref_id, object).await?;
    }
    Ok(())
}

pub async fn insert_object_reference<C>(
    conn: &C,
    ref_kind: &str,
    ref_id: &str,
    object: &SourceBlob,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::object_reference::Model {
        object_key: object.object_key.clone(),
        ref_kind: ref_kind.to_string(),
        ref_id: ref_id.to_string(),
    }
    .into_active_model()
    .insert(conn)
    .await
    .map_err(ApiError::internal)?;
    Ok(())
}

pub async fn delete_object_reference<C>(
    conn: &C,
    ref_kind: &str,
    ref_id: &str,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    replace_object_reference(conn, ref_kind, ref_id, None).await
}

pub async fn delete_object_references_for_objects<C>(
    conn: &C,
    objects: impl IntoIterator<Item = &SourceBlob>,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let keys = objects
        .into_iter()
        .map(|object| object.object_key.clone())
        .collect::<Vec<_>>();
    if keys.is_empty() {
        return Ok(());
    }
    entities::object_reference::Entity::delete_many()
        .filter(entities::object_reference::Column::ObjectKey.is_in(keys))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

pub async fn referenced_object_keys<C>(
    conn: &C,
) -> Result<std::collections::BTreeSet<String>, ApiError>
where
    C: ConnectionTrait,
{
    Ok(entities::object_reference::Entity::find()
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|row| row.object_key)
        .collect())
}
