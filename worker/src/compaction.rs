use crate::git_repo::build_compacted_pack;
use scope_domain::store::{GitHead, GitSegment, SourceBlob};
use scope_git::{GitStorageLimits, materialize_compacted_git_segment};
use scope_git_process::ProcessError;
use scope_object_store::ObjectStore;
use scope_postgres::db::MetadataStore;
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
    let now_unix = super::unix_now()?;
    let Some(candidate) = metadata
        .jobs()
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
                        now_unix,
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
        .jobs()
        .replace_git_segments_with_compaction(
            &candidate.repo_id,
            &candidate.head.manifest.content_ref,
            new_head,
            new_segment,
            now_unix,
            &crate::generate_persistence_id,
        )
        .await
    {
        Ok(true) => Ok(CompactionOutcome::Applied),
        Ok(false) => Ok(CompactionOutcome::Stale),
        Err(error) => {
            if let Err(queue_error) = metadata
                .cleanup()
                .queue_pending_source_blob_deletions(
                    stored_objects.to_vec(),
                    now_unix,
                    &crate::generate_persistence_id,
                )
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
    now_unix: u64,
) -> anyhow::Result<()> {
    if let Err(queue_error) = metadata
        .cleanup()
        .queue_pending_source_blob_deletions(
            objects.to_vec(),
            now_unix,
            &crate::generate_persistence_id,
        )
        .await
    {
        let mut delete_errors = Vec::new();
        for object in objects {
            if let Err(error) = object_store.delete(&scope_object_store::object_key(object)) {
                delete_errors.push(format!(
                    "{}: {}",
                    scope_object_store::object_key(object),
                    error.message
                ));
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
    candidate: &scope_postgres::db::GitCompactionCandidate,
    storage_limits: GitStorageLimits,
    timeout: Duration,
) -> Result<(GitHead, GitSegment), CompactedSegmentBuildFailure> {
    let pack = build_compacted_pack(object_store, candidate, storage_limits, timeout)?;
    match materialize_compacted_git_segment(object_store, &pack, &candidate.head, storage_limits) {
        Ok(stored) => Ok((stored.head, stored.segment)),
        Err(failure) => {
            let (error, orphan_objects) = failure.into_parts();
            Err(CompactedSegmentBuildFailure {
                error: anyhow::Error::new(error),
                orphan_objects,
            })
        }
    }
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
