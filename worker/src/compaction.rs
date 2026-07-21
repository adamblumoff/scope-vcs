use crate::git_repo::build_compacted_pack;
use scope_core::{
    db::MetadataStore,
    domain::store::{GitHead, GitSegment, SourceBlob},
    git_segments::{GitSegmentManifest, GitStorageLimits},
    object_store::{ObjectStore, ensure_object_size, put_repo_object},
};
use scope_git_process::ProcessError;
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CompactionOutcome {
    NoCandidate,
    Applied,
    Stale,
    Refused(String),
}

pub(crate) async fn compact_one_git_repository(
    metadata: &MetadataStore,
    object_store: &dyn ObjectStore,
    minimum_segments: usize,
    storage_limits: GitStorageLimits,
    timeout: Duration,
) -> anyhow::Result<CompactionOutcome> {
    let Some(candidate) = metadata
        .git_compaction_candidate(minimum_segments as u64)
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?
    else {
        return Ok(CompactionOutcome::NoCandidate);
    };
    let (new_head, new_segment) =
        match build_compacted_segment(object_store, &candidate, storage_limits, timeout) {
            Ok(compaction) => compaction,
            Err(failure) => {
                if !failure.orphan_objects.is_empty() {
                    queue_or_delete_failed_compaction_objects(
                        metadata,
                        object_store,
                        &failure.orphan_objects,
                    )
                    .await?;
                }
                if is_bounded_refusal(&failure.error) {
                    return Ok(CompactionOutcome::Refused(failure.error.to_string()));
                }
                return Err(failure.error);
            }
        };
    let stored_objects = [new_segment.object.clone(), new_segment.manifest.clone()];
    match metadata
        .replace_git_segments_with_compaction(
            &candidate.repo_id,
            &candidate.head.manifest.object_key,
            new_head,
            new_segment,
        )
        .await
    {
        Ok(true) => Ok(CompactionOutcome::Applied),
        Ok(false) => Ok(CompactionOutcome::Stale),
        Err(error) => {
            if let Err(queue_error) = metadata
                .queue_pending_source_blob_deletions(stored_objects.to_vec())
                .await
            {
                return Err(anyhow::anyhow!(
                    "persisting Git compaction may have committed: {}; cleanup queue failed, retaining objects for reconciliation: {}",
                    error.message,
                    queue_error.message
                ));
            }
            Err(anyhow::anyhow!(
                "persisting Git compaction failed: {}",
                error.message
            ))
        }
    }
}

async fn queue_or_delete_failed_compaction_objects(
    metadata: &MetadataStore,
    object_store: &dyn ObjectStore,
    objects: &[SourceBlob],
) -> anyhow::Result<()> {
    if let Err(queue_error) = metadata
        .queue_pending_source_blob_deletions(objects.to_vec())
        .await
    {
        let mut delete_errors = Vec::new();
        for object in objects {
            if let Err(error) = object_store.delete(&object.object_key) {
                delete_errors.push(format!("{}: {}", object.object_key, error.message));
            }
        }
        if !delete_errors.is_empty() {
            anyhow::bail!(
                "cleanup queue failed: {}; direct cleanup failed: {}",
                queue_error.message,
                delete_errors.join(", ")
            );
        }
    }
    Ok(())
}

struct CompactedSegmentBuildFailure {
    error: anyhow::Error,
    orphan_objects: Vec<SourceBlob>,
}

impl From<anyhow::Error> for CompactedSegmentBuildFailure {
    fn from(error: anyhow::Error) -> Self {
        Self {
            error,
            orphan_objects: Vec::new(),
        }
    }
}

fn build_compacted_segment(
    object_store: &dyn ObjectStore,
    candidate: &scope_core::db::GitCompactionCandidate,
    storage_limits: GitStorageLimits,
    timeout: Duration,
) -> Result<(GitHead, GitSegment), CompactedSegmentBuildFailure> {
    let pack = build_compacted_pack(object_store, candidate, storage_limits, timeout)?;
    ensure_object_size(
        "write",
        "Git compacted segment",
        pack.len(),
        storage_limits.max_object_bytes(),
    )
    .map_err(|error| anyhow::anyhow!(error.message))?;
    let segment = put_repo_object(object_store, &candidate.repo_id, "git-segments", &pack)
        .map_err(|error| CompactedSegmentBuildFailure::from(anyhow::anyhow!(error.message)))?;
    let manifest = GitSegmentManifest::new(candidate.head.head_oid.clone(), None, segment.clone());
    let manifest_bytes = manifest
        .encode()
        .map_err(|error| CompactedSegmentBuildFailure {
            error: anyhow::anyhow!(error.message),
            orphan_objects: vec![segment.clone()],
        })?;
    ensure_object_size(
        "write",
        "Git compacted manifest",
        manifest_bytes.len(),
        storage_limits.max_object_bytes(),
    )
    .map_err(|error| CompactedSegmentBuildFailure {
        error: anyhow::anyhow!(error.message),
        orphan_objects: vec![segment.clone()],
    })?;
    let mut manifest_object = match put_repo_object(
        object_store,
        &candidate.repo_id,
        "git-manifests",
        &manifest_bytes,
    ) {
        Ok(object) => object,
        Err(error) => {
            return Err(CompactedSegmentBuildFailure {
                error: anyhow::anyhow!(error.message),
                orphan_objects: vec![segment],
            });
        }
    };
    manifest_object.git_oid = candidate.head.head_oid.clone();
    Ok((
        GitHead {
            head_oid: candidate.head.head_oid.clone(),
            segment_sequence: 1,
            change_version: candidate.head.change_version,
            manifest: manifest_object.clone(),
        },
        GitSegment {
            sequence: 1,
            base_oid: None,
            head_oid: candidate.head.head_oid.clone(),
            object: segment,
            manifest: manifest_object,
        },
    ))
}

fn is_bounded_refusal(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<ProcessError>()
        .is_some_and(|error| error.is_timeout() || error.is_stdout_limit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_timeout_and_output_limit_are_safe_refusals() {
        let timeout = anyhow::Error::new(ProcessError::TimedOut {
            action: "git fsck".to_string(),
            timeout_ms: 1,
            diagnostic: String::new(),
        });
        let oversized = anyhow::Error::new(ProcessError::StdoutLimitExceeded {
            action: "git pack-objects".to_string(),
            max_stdout_bytes: 4,
            diagnostic: String::new(),
        });

        assert!(is_bounded_refusal(&timeout));
        assert!(is_bounded_refusal(&oversized));
        assert!(!is_bounded_refusal(&anyhow::anyhow!("ordinary failure")));
    }
}
