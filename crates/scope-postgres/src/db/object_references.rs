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

pub async fn delete_repository_object_references<C>(
    conn: &C,
    repo_id: &str,
    request_ids: &[String],
    change_block_ids: &[String],
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    entities::object_reference::Entity::delete_many()
        .filter(entities::object_reference::Column::RefKind.eq("git_manifest"))
        .filter(entities::object_reference::Column::RefId.eq(repo_id.to_string()))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;

    entities::object_reference::Entity::delete_many()
        .filter(entities::object_reference::Column::RefKind.is_in([
            "git_segment",
            "git_segment_manifest",
            "file_change",
            "visibility_event",
            "push_trigger_source",
        ]))
        .filter(entities::object_reference::Column::RefId.starts_with(format!("{repo_id}:")))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;

    if !request_ids.is_empty() {
        entities::object_reference::Entity::delete_many()
            .filter(entities::object_reference::Column::RefKind.eq("request_snapshot"))
            .filter(entities::object_reference::Column::RefId.is_in(request_ids.to_vec()))
            .exec(conn)
            .await
            .map_err(PostgresError::internal)?;
    }
    if !change_block_ids.is_empty() {
        entities::object_reference::Entity::delete_many()
            .filter(entities::object_reference::Column::RefKind.eq("request_change_block_snapshot"))
            .filter(entities::object_reference::Column::RefId.is_in(change_block_ids.to_vec()))
            .exec(conn)
            .await
            .map_err(PostgresError::internal)?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{MetadataStore, TestDatabaseTarget};
    use scope_domain::store::DEFAULT_GIT_FILE_MODE;

    #[tokio::test]
    async fn repository_deletion_preserves_shared_content_references_owned_by_another_repo() {
        let target = TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let shared = SourceBlob {
            content_ref: ContentRef::git_manifest_sha256("shared-manifest"),
            sha256: "shared-manifest".to_string(),
            git_oid: "1111111111111111111111111111111111111111".to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 42,
        };
        insert_object_reference(store.db.as_ref(), "git_manifest", "owner-a/repo", &shared)
            .await
            .unwrap();
        insert_object_reference(store.db.as_ref(), "git_manifest", "owner-b/repo", &shared)
            .await
            .unwrap();

        delete_repository_object_references(store.db.as_ref(), "owner-a/repo", &[], &[])
            .await
            .unwrap();

        let rows = entities::object_reference::Entity::find()
            .all(store.db.as_ref())
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ref_kind, "git_manifest");
        assert_eq!(rows[0].ref_id, "owner-b/repo");
        assert_eq!(
            decode_content_ref(&rows[0].object_key).unwrap(),
            shared.content_ref
        );
    }
}
