use super::entities;
use crate::error::PostgresError;
use scope_domain::{content_ref::ContentRef, store::SourceBlob};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
};

fn encode_content_ref(content_ref: &ContentRef) -> Result<String, PostgresError> {
    serde_json::to_string(content_ref).map_err(PostgresError::internal)
}

fn decode_content_ref(encoded: &str) -> Result<ContentRef, PostgresError> {
    serde_json::from_str(encoded).map_err(PostgresError::internal)
}

pub async fn replace_object_reference<C>(
    conn: &C,
    ref_kind: &str,
    ref_id: &str,
    object: Option<&SourceBlob>,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    entities::object_reference::Entity::delete_many()
        .filter(entities::object_reference::Column::RefKind.eq(ref_kind.to_string()))
        .filter(entities::object_reference::Column::RefId.eq(ref_id.to_string()))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
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
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    entities::object_reference::Model {
        object_key: encode_content_ref(&object.content_ref)?,
        ref_kind: ref_kind.to_string(),
        ref_id: ref_id.to_string(),
    }
    .into_active_model()
    .insert(conn)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub async fn delete_object_reference<C>(
    conn: &C,
    ref_kind: &str,
    ref_id: &str,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    replace_object_reference(conn, ref_kind, ref_id, None).await
}

pub async fn delete_object_references_for_objects<C>(
    conn: &C,
    objects: impl IntoIterator<Item = &SourceBlob>,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let content_refs = objects
        .into_iter()
        .map(|object| encode_content_ref(&object.content_ref))
        .collect::<Result<Vec<_>, _>>()?;
    if content_refs.is_empty() {
        return Ok(());
    }
    entities::object_reference::Entity::delete_many()
        .filter(entities::object_reference::Column::ObjectKey.is_in(content_refs))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    Ok(())
}

pub async fn referenced_content_refs<C>(
    conn: &C,
) -> Result<std::collections::BTreeSet<ContentRef>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::object_reference::Entity::find()
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| decode_content_ref(&row.object_key))
        .collect::<Result<_, _>>()
}
