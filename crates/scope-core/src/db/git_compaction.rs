use super::{
    MetadataStore, acquire_aggregate_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    entities,
    object_references::{
        delete_object_reference, insert_object_reference, replace_object_reference,
    },
};
use crate::{
    domain::store::{GitHead, GitSegment, SourceBlob},
    error::ApiError,
    git_segments::{
        GIT_BLOB_REFERENCE_PREFIX, git_blob_reference, repoint_git_blob_reference,
        validate_compacted_replacement,
    },
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, Set, TransactionTrait,
};

#[derive(Clone, Debug)]
pub struct GitCompactionCandidate {
    pub repo_id: String,
    pub owner: String,
    pub name: String,
    pub head: GitHead,
    pub segments: Vec<GitSegment>,
}

impl MetadataStore {
    pub async fn git_compaction_candidate(
        &self,
        minimum_segments: u64,
    ) -> Result<Option<GitCompactionCandidate>, ApiError> {
        let minimum_segments = i64::try_from(minimum_segments).map_err(|_| {
            ApiError::internal_message("Git compaction segment threshold exceeds bigint")
        })?;
        let Some(head_row) = entities::git_head::Entity::find()
            .filter(entities::git_head::Column::SegmentSequence.gte(minimum_segments))
            .order_by_desc(entities::git_head::Column::SegmentSequence)
            .one(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?
        else {
            return Ok(None);
        };
        let repo_id = head_row.repo_id.clone();
        let repo = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::internal_message("Git head has no repository"))?;
        let segments = entities::git_segment::Entity::find()
            .filter(entities::git_segment::Column::RepoId.eq(repo_id.clone()))
            .order_by_asc(entities::git_segment::Column::Sequence)
            .all(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?
            .into_iter()
            .map(entities::git_segment::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        if segments.len() < minimum_segments as usize {
            return Ok(None);
        }
        Ok(Some(GitCompactionCandidate {
            repo_id,
            owner: repo.owner_handle,
            name: repo.name,
            head: head_row.try_into_domain()?,
            segments,
        }))
    }

    pub async fn replace_git_segments_with_compaction(
        &self,
        repo_id: &str,
        expected_manifest_key: &str,
        new_head: GitHead,
        new_segment: GitSegment,
    ) -> Result<bool, ApiError> {
        validate_compacted_replacement(&new_head, &new_segment)
            .map_err(|error| ApiError::internal_message(error.to_string()))?;
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "repository", repo_id).await?;
        let current = entities::git_head::Entity::find_by_id(repo_id.to_string())
            .one(&tx)
            .await
            .map_err(ApiError::internal)?;
        let Some(current) = current else {
            queue_pending_source_blob_deletion_rows(
                &tx,
                [new_head.manifest.clone(), new_segment.object.clone()],
            )
            .await?;
            tx.commit().await.map_err(ApiError::internal)?;
            return Ok(false);
        };
        if current.manifest_object_key != expected_manifest_key {
            queue_pending_source_blob_deletion_rows(
                &tx,
                [new_head.manifest.clone(), new_segment.object.clone()],
            )
            .await?;
            tx.commit().await.map_err(ApiError::internal)?;
            return Ok(false);
        }
        let current_head = current.clone().try_into_domain()?;
        if new_head.head_oid != current_head.head_oid
            || new_head.change_version != current_head.change_version
            || new_segment.head_oid != current_head.head_oid
        {
            return Err(ApiError::internal_message(
                "Git compaction cannot change the visible repository head",
            ));
        }
        let old_segments = entities::git_segment::Entity::find()
            .filter(entities::git_segment::Column::RepoId.eq(repo_id.to_string()))
            .order_by_asc(entities::git_segment::Column::Sequence)
            .all(&tx)
            .await
            .map_err(ApiError::internal)?
            .into_iter()
            .map(entities::git_segment::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;

        entities::git_head::Entity::delete_by_id(repo_id.to_string())
            .exec(&tx)
            .await
            .map_err(ApiError::internal)?;
        entities::git_head::Model::from_domain(repo_id, &new_head)?
            .into_active_model()
            .insert(&tx)
            .await
            .map_err(ApiError::internal)?;
        replace_object_reference(&tx, "git_manifest", repo_id, Some(&new_head.manifest)).await?;
        repoint_repository_git_blob_references(&tx, repo_id, &new_head.manifest).await?;

        for segment in &old_segments {
            let ref_id = format!("{repo_id}:{}", segment.sequence);
            delete_object_reference(&tx, "git_segment", &ref_id).await?;
            delete_object_reference(&tx, "git_segment_manifest", &ref_id).await?;
        }
        entities::git_segment::Entity::delete_many()
            .filter(entities::git_segment::Column::RepoId.eq(repo_id.to_string()))
            .exec(&tx)
            .await
            .map_err(ApiError::internal)?;
        entities::git_segment::Model::from_domain(repo_id, &new_segment)?
            .into_active_model()
            .insert(&tx)
            .await
            .map_err(ApiError::internal)?;
        let ref_id = format!("{repo_id}:{}", new_segment.sequence);
        insert_object_reference(&tx, "git_segment", &ref_id, &new_segment.object).await?;
        insert_object_reference(&tx, "git_segment_manifest", &ref_id, &new_segment.manifest)
            .await?;

        let old_objects = old_segments
            .into_iter()
            .flat_map(|segment| [segment.object, segment.manifest])
            .filter(|object| {
                object.object_key != new_segment.object.object_key
                    && object.object_key != new_segment.manifest.object_key
            })
            .collect::<Vec<SourceBlob>>();
        queue_pending_source_blob_deletion_rows(&tx, old_objects).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(true)
    }
}

async fn repoint_repository_git_blob_references<C>(
    conn: &C,
    repo_id: &str,
    manifest: &SourceBlob,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    for row in entities::live_file::Entity::find()
        .filter(entities::live_file::Column::RepoId.eq(repo_id.to_string()))
        .all(conn)
        .await
        .map_err(ApiError::internal)?
    {
        let mut content: SourceBlob =
            serde_json::from_value(row.content.clone()).map_err(ApiError::internal)?;
        if repoint_git_blob_reference(&mut content, manifest)? {
            let mut active = row.into_active_model();
            active.content = Set(serde_json::to_value(content).map_err(ApiError::internal)?);
            active.update(conn).await.map_err(ApiError::internal)?;
        }
    }
    for row in entities::file_change::Entity::find()
        .filter(entities::file_change::Column::RepoId.eq(repo_id.to_string()))
        .all(conn)
        .await
        .map_err(ApiError::internal)?
    {
        let mut old_content = row.old_content.clone();
        let mut new_content = row.new_content.clone();
        let changed = repoint_optional_content(&mut old_content, manifest)?
            | repoint_optional_content(&mut new_content, manifest)?;
        if changed {
            let mut active = row.into_active_model();
            active.old_content = Set(old_content);
            active.new_content = Set(new_content);
            active.update(conn).await.map_err(ApiError::internal)?;
        }
    }
    for row in entities::visibility_event::Entity::find()
        .filter(entities::visibility_event::Column::RepoId.eq(repo_id.to_string()))
        .all(conn)
        .await
        .map_err(ApiError::internal)?
    {
        let mut current_content = row.current_content.clone();
        if repoint_optional_content(&mut current_content, manifest)? {
            let mut active = row.into_active_model();
            active.current_content = Set(current_content);
            active.update(conn).await.map_err(ApiError::internal)?;
        }
    }
    for row in entities::projection_file::Entity::find()
        .filter(entities::projection_file::Column::RepoId.eq(repo_id.to_string()))
        .filter(
            entities::projection_file::Column::ObjectKey
                .starts_with(GIT_BLOB_REFERENCE_PREFIX.to_string()),
        )
        .all(conn)
        .await
        .map_err(ApiError::internal)?
    {
        let size_bytes = u64::try_from(row.size_bytes)
            .map_err(|_| ApiError::internal_message("projection file size cannot be negative"))?;
        let object_key = git_blob_reference(
            manifest,
            row.oid.clone(),
            row.git_file_mode.clone(),
            size_bytes,
        )?
        .object_key;
        let mut active = row.into_active_model();
        active.object_key = Set(object_key);
        active.update(conn).await.map_err(ApiError::internal)?;
    }
    Ok(())
}

fn repoint_optional_content(
    value: &mut Option<serde_json::Value>,
    manifest: &SourceBlob,
) -> Result<bool, ApiError> {
    let Some(encoded) = value.as_mut() else {
        return Ok(false);
    };
    let mut content: SourceBlob =
        serde_json::from_value(encoded.clone()).map_err(ApiError::internal)?;
    if !repoint_git_blob_reference(&mut content, manifest)? {
        return Ok(false);
    }
    *encoded = serde_json::to_value(content).map_err(ApiError::internal)?;
    Ok(true)
}
