use crate::git_repo::{CompactionPackMetrics, build_compacted_pack};
use scope_domain::store::{GitPackSpan, SourceBlob};
use scope_git::{GitStorageLimits, store_compacted_git_pack};
use scope_git_process::ProcessError;
use scope_object_store::ObjectStore;
use scope_postgres::db::MetadataStore;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    health::WorkerHealth,
    settings::{WorkerRole, WorkerSettings},
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CompactionOutcome {
    NoJob,
    Drained,
    Applied,
    Stale,
    Refused(String),
}

pub(crate) async fn run(
    metadata: MetadataStore,
    object_store: Arc<dyn ObjectStore>,
    settings: WorkerSettings,
    health: WorkerHealth,
) -> anyhow::Result<()> {
    loop {
        if !super::schema_ready_or_wait(&metadata, &health).await {
            return Ok(());
        }
        let made_progress = match compact_one_git_repository(
            &metadata,
            object_store.as_ref(),
            &settings.worker_id,
            settings.git_compaction_spans,
            settings.git_storage_limits,
            settings.git_compaction_timeout,
        )
        .await
        {
            Ok(
                CompactionOutcome::Applied | CompactionOutcome::Stale | CompactionOutcome::Drained,
            ) => true,
            Ok(CompactionOutcome::NoJob) => false,
            Ok(CompactionOutcome::Refused(reason)) => {
                tracing::warn!(reason, "Git compaction refused bounded replacement");
                false
            }
            Err(error) => {
                tracing::error!(error = %error, "Git compaction failed; retry is scheduled");
                false
            }
        };
        health.mark_poll_succeeded(WorkerRole::Compaction, super::unix_now()?);
        if made_progress {
            continue;
        }
        if super::wait_or_shutdown(settings.poll_interval).await {
            return Ok(());
        }
    }
}

pub(crate) async fn compact_one_git_repository(
    metadata: &MetadataStore,
    object_store: &dyn ObjectStore,
    worker_id: &str,
    minimum_spans: usize,
    storage_limits: GitStorageLimits,
    timeout: Duration,
) -> anyhow::Result<CompactionOutcome> {
    let attempt_started = Instant::now();
    let claim_now_unix = super::unix_now()?;
    let candidate_started = Instant::now();
    let lease_seconds = timeout.as_secs().saturating_add(30);
    let Some(claim) = metadata
        .jobs()
        .claim_git_compaction(
            worker_id,
            minimum_spans as u64,
            claim_now_unix,
            lease_seconds,
            &crate::generate_persistence_id,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?
    else {
        return Ok(CompactionOutcome::NoJob);
    };
    let Some(candidate) = claim.candidate.as_ref() else {
        metadata
            .jobs()
            .complete_git_compaction_claim(&claim, super::unix_now()?)
            .await
            .map_err(|error| anyhow::anyhow!(error.message))?;
        return Ok(CompactionOutcome::Drained);
    };
    let candidate_query_ms = elapsed_ms(candidate_started);
    let built = match build_compacted_span(object_store, candidate, storage_limits, timeout) {
        Ok(compaction) => compaction,
        Err(failure) => {
            let failure_now_unix = super::unix_now()?;
            metadata
                .jobs()
                .fail_git_compaction_claim(&claim, &failure.error.to_string(), failure_now_unix)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?;
            if !failure.orphan_objects.is_empty() {
                queue_or_delete_failed_compaction_objects(
                    metadata,
                    object_store,
                    &failure.orphan_objects,
                    failure_now_unix,
                )
                .await?;
            }
            if is_bounded_refusal(&failure.error) {
                return Ok(CompactionOutcome::Refused(failure.error.to_string()));
            }
            return Err(failure.error);
        }
    };
    let stored_objects = [built.replacement.object.clone()];
    let persist_now_unix = super::unix_now()?;
    let persist_started = Instant::now();
    let persisted = metadata
        .jobs()
        .replace_git_pack_spans_with_compaction(
            &candidate.repo_id,
            &candidate.spans,
            built.replacement,
            persist_now_unix,
            &crate::generate_persistence_id,
        )
        .await;
    let persist_ms = elapsed_ms(persist_started);
    match persisted {
        Ok(applied) => {
            metadata
                .jobs()
                .continue_git_compaction_claim(&claim, super::unix_now()?)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?;
            let outcome = if applied { "applied" } else { "stale" };
            log_compaction_attempt(
                &claim,
                &candidate,
                &built.metrics,
                candidate_query_ms,
                persist_ms,
                elapsed_ms(attempt_started),
                outcome,
            );
            Ok(if applied {
                CompactionOutcome::Applied
            } else {
                CompactionOutcome::Stale
            })
        }
        Err(error) => {
            let failure_now_unix = super::unix_now()?;
            metadata
                .jobs()
                .fail_git_compaction_claim(&claim, &error.message, failure_now_unix)
                .await
                .map_err(|claim_error| anyhow::anyhow!(claim_error.message))?;
            if let Err(queue_error) = metadata
                .cleanup()
                .queue_pending_source_blob_deletions(
                    stored_objects.to_vec(),
                    failure_now_unix,
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

fn log_compaction_attempt(
    claim: &scope_postgres::db::GitCompactionClaim,
    candidate: &scope_postgres::db::GitCompactionCandidate,
    metrics: &CompactionMetrics,
    candidate_query_ms: u64,
    persist_ms: u64,
    total_ms: u64,
    outcome: &str,
) {
    tracing::info!(
        outcome,
        repo_id = %candidate.repo_id,
        owner = %candidate.owner,
        repo = %candidate.name,
        target_sequence = claim.target_sequence,
        scheduler_attempts = claim.attempts,
        scheduler_queue_delay_ms = claim.queue_delay_ms,
        source_span_count = metrics.pack.source_span_count,
        source_pack_bytes = metrics.pack.source_pack_bytes,
        predecessor_pack_bytes = metrics.pack.predecessor_pack_bytes,
        compacted_bytes = metrics.pack.compacted_bytes,
        candidate_query_ms,
        init_ms = metrics.pack.init_ms,
        download_ms = metrics.pack.download_ms,
        index_ms = metrics.pack.index_ms,
        update_ref_ms = metrics.pack.update_ref_ms,
        connectivity_check_ms = metrics.pack.connectivity_check_ms,
        pack_ms = metrics.pack.pack_ms,
        pack_total_ms = metrics.pack.total_ms,
        store_ms = metrics.store_ms,
        persist_ms,
        total_ms,
        "Git compaction attempt completed"
    );
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
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

struct CompactedSpanBuildFailure {
    error: anyhow::Error,
    orphan_objects: Vec<SourceBlob>,
}

struct BuiltCompaction {
    replacement: GitPackSpan,
    metrics: CompactionMetrics,
}

struct CompactionMetrics {
    pack: CompactionPackMetrics,
    store_ms: u64,
}

impl From<anyhow::Error> for CompactedSpanBuildFailure {
    fn from(error: anyhow::Error) -> Self {
        Self {
            error,
            orphan_objects: Vec::new(),
        }
    }
}

fn build_compacted_span(
    object_store: &dyn ObjectStore,
    candidate: &scope_postgres::db::GitCompactionCandidate,
    storage_limits: GitStorageLimits,
    timeout: Duration,
) -> Result<BuiltCompaction, CompactedSpanBuildFailure> {
    let pack = build_compacted_pack(object_store, candidate, storage_limits, timeout)?;
    let store_started = Instant::now();
    match store_compacted_git_pack(object_store, &pack.bytes, storage_limits) {
        Ok(object) => {
            let first = candidate
                .spans
                .first()
                .expect("persistence returns nonempty compaction candidates");
            let last = candidate
                .spans
                .last()
                .expect("persistence returns nonempty compaction candidates");
            let mut replacement = GitPackSpan {
                first_sequence: first.first_sequence,
                last_sequence: last.last_sequence,
                geometric_tier: 0,
                base_oid: first.base_oid.clone(),
                head_oid: last.head_oid.clone(),
                object,
            };
            replacement.geometric_tier = replacement
                .expected_geometric_tier()
                .map_err(|error| CompactedSpanBuildFailure::from(anyhow::Error::new(error)))?;
            Ok(BuiltCompaction {
                replacement,
                metrics: CompactionMetrics {
                    pack: pack.metrics,
                    store_ms: elapsed_ms(store_started),
                },
            })
        }
        Err(failure) => {
            let (error, orphan_objects) = failure.into_parts();
            Err(CompactedSpanBuildFailure {
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
