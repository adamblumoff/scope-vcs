use super::{
    GeneratedIdSource, JobStore, acquire_aggregate_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    entities,
    object_references::{
        delete_object_reference, insert_object_reference, replace_object_reference,
    },
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    TransactionTrait,
};
use {
    crate::error::PostgresError,
    scope_domain::store::{GitHead, GitSegment, SourceBlob},
    scope_git::validate_compacted_replacement,
};

#[derive(Clone, Debug)]
pub struct GitCompactionCandidate {
    pub repo_id: String,
    pub owner: String,
    pub name: String,
    pub head: GitHead,
    pub segments: Vec<GitSegment>,
}

impl JobStore {
    pub async fn git_compaction_candidate(
        &self,
        minimum_segments: u64,
    ) -> Result<Option<GitCompactionCandidate>, PostgresError> {
        let minimum_segments = i64::try_from(minimum_segments).map_err(|_| {
            PostgresError::internal_message("Git compaction segment threshold exceeds bigint")
        })?;
        let Some(head_row) = entities::git_head::Entity::find()
            .filter(entities::git_head::Column::SegmentSequence.gte(minimum_segments))
            .order_by_desc(entities::git_head::Column::SegmentSequence)
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
        else {
            return Ok(None);
        };
        let repo_id = head_row.repo_id.clone();
        let repo = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("Git head has no repository"))?;
        let segments = entities::git_segment::Entity::find()
            .filter(entities::git_segment::Column::RepoId.eq(repo_id.clone()))
            .order_by_asc(entities::git_segment::Column::Sequence)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
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
        expected_manifest_ref: &scope_domain::content_ref::ContentRef,
        new_head: GitHead,
        new_segment: GitSegment,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<bool, PostgresError> {
        validate_compacted_replacement(&new_head, &new_segment)
            .map_err(|error| PostgresError::internal_message(error.to_string()))?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", repo_id).await?;
        let current = entities::git_head::Entity::find_by_id(repo_id.to_string())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let Some(current) = current else {
            queue_pending_source_blob_deletion_rows(
                &tx,
                [new_head.manifest.clone(), new_segment.object.clone()],
                now_unix,
                generated_ids,
            )
            .await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(false);
        };
        let current_head = current.clone().try_into_domain()?;
        if &current_head.manifest.content_ref != expected_manifest_ref {
            queue_pending_source_blob_deletion_rows(
                &tx,
                [new_head.manifest.clone(), new_segment.object.clone()],
                now_unix,
                generated_ids,
            )
            .await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(false);
        }
        if new_head.head_oid != current_head.head_oid
            || new_head.change_version != current_head.change_version
            || new_segment.head_oid != current_head.head_oid
        {
            return Err(PostgresError::internal_message(
                "Git compaction cannot change the visible repository head",
            ));
        }
        let old_segments = entities::git_segment::Entity::find()
            .filter(entities::git_segment::Column::RepoId.eq(repo_id.to_string()))
            .order_by_asc(entities::git_segment::Column::Sequence)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::git_segment::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;

        entities::git_head::Entity::delete_by_id(repo_id.to_string())
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::git_head::Model::from_domain(repo_id, &new_head)?
            .into_active_model()
            .insert(&tx)
            .await
            .map_err(PostgresError::internal)?;
        replace_object_reference(&tx, "git_manifest", repo_id, Some(&new_head.manifest)).await?;

        for segment in &old_segments {
            let ref_id = format!("{repo_id}:{}", segment.sequence);
            delete_object_reference(&tx, "git_segment", &ref_id).await?;
            delete_object_reference(&tx, "git_segment_manifest", &ref_id).await?;
        }
        entities::git_segment::Entity::delete_many()
            .filter(entities::git_segment::Column::RepoId.eq(repo_id.to_string()))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::git_segment::Model::from_domain(repo_id, &new_segment)?
            .into_active_model()
            .insert(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let ref_id = format!("{repo_id}:{}", new_segment.sequence);
        insert_object_reference(&tx, "git_segment", &ref_id, &new_segment.object).await?;
        insert_object_reference(&tx, "git_segment_manifest", &ref_id, &new_segment.manifest)
            .await?;

        let old_objects = old_segments
            .into_iter()
            .flat_map(|segment| [segment.object, segment.manifest])
            .filter(|object| {
                object.content_ref != new_segment.object.content_ref
                    && object.content_ref != new_segment.manifest.content_ref
            })
            .collect::<Vec<SourceBlob>>();
        queue_pending_source_blob_deletion_rows(&tx, old_objects, now_unix, generated_ids).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(true)
    }
}
