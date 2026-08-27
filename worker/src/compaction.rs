use crate::git_repo::{CompactedPackFailure, CompactionPackMetrics, build_compacted_pack};
use scope_domain::repository::git::GitPackSpan;
use scope_git::GitStorageLimits;
use scope_git_process::ProcessError;
use scope_git_storage::{
    ENCODING_VERSION, GitSegmentIngestTimings, GitSegmentReservation, GitSegmentStore,
    StagedGitSegment,
};
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
    segment_store: Arc<GitSegmentStore>,
    settings: WorkerSettings,
    health: WorkerHealth,
) -> anyhow::Result<()> {
    loop {
        if !super::schema_ready_or_wait(&metadata, &health).await {
            return Ok(());
        }
        let made_progress = match compact_one_git_repository(
            &metadata,
            Arc::clone(&segment_store),
            &settings.worker_id,
            settings.git_compaction_spans,
            settings.git_storage_limits,
            settings.git_compaction_timeout,
            settings.data_dir.clone(),
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
    segment_store: Arc<GitSegmentStore>,
    worker_id: &str,
    minimum_spans: usize,
    storage_limits: GitStorageLimits,
    timeout: Duration,
    data_dir: std::path::PathBuf,
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
    let reservation = segment_store
        .reserve(&candidate.repo_id)
        .map_err(anyhow::Error::new)?;
    metadata
        .repositories()
        .begin_git_segment_upload(
            &candidate.repo_id,
            &reservation.segment_id,
            &reservation.object_key,
            ENCODING_VERSION,
            super::unix_now()?,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let build_candidate = candidate.clone();
    let build_store = Arc::clone(&segment_store);
    let build_reservation = reservation.clone();
    let mut build = tokio::spawn(async move {
        build_compacted_span(
            build_store,
            &build_candidate,
            build_reservation,
            storage_limits,
            timeout,
            data_dir,
        )
        .await
    });
    let renewal_interval = Duration::from_secs((lease_seconds / 3).max(1));
    let mut renewal = tokio::time::interval(renewal_interval);
    renewal.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    renewal.tick().await;
    let built = loop {
        tokio::select! {
            result = &mut build => {
                break match result {
                    Ok(result) => result,
                    Err(error) => Err(CompactedPackFailure::from(anyhow::anyhow!(
                        "Git compaction task failed: {error}"
                    ))),
                };
            }
            _ = renewal.tick() => {
                match metadata.jobs().renew_git_compaction_claim(
                    &claim,
                    super::unix_now()?,
                    lease_seconds,
                ).await {
                    Ok(true) => {}
                    Ok(false) => tracing::warn!(
                        target_sequence = claim.target_sequence,
                        "Git compaction lease was lost while external work was still running"
                    ),
                    Err(error) => tracing::warn!(
                        error = %error.message,
                        target_sequence = claim.target_sequence,
                        "Git compaction lease renewal failed"
                    ),
                }
            }
        }
    };
    let built = match built {
        Ok(compaction) => compaction,
        Err(failure) => {
            let failure_now_unix = super::unix_now()?;
            if let Err(cleanup_error) = abandon_upload(
                metadata,
                segment_store.as_ref(),
                &candidate.repo_id,
                &reservation,
                failure_now_unix,
            )
            .await
            {
                tracing::warn!(
                    error = %cleanup_error,
                    segment_id = reservation.segment_id,
                    "failed to discard unsuccessful Git compaction upload"
                );
            }
            if is_bounded_refusal(&failure.error) {
                metadata
                    .jobs()
                    .complete_git_compaction_claim(&claim, failure_now_unix)
                    .await
                    .map_err(|error| anyhow::anyhow!(error.message))?;
                return Ok(CompactionOutcome::Refused(failure.error.to_string()));
            }
            metadata
                .jobs()
                .fail_git_compaction_claim(&claim, &failure.error.to_string(), failure_now_unix)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?;
            return Err(failure.error);
        }
    };
    let final_renewal = metadata
        .jobs()
        .renew_git_compaction_claim(&claim, super::unix_now()?, lease_seconds)
        .await;
    if !matches!(final_renewal, Ok(true)) {
        let cleanup_now_unix = super::unix_now()?;
        if let Err(cleanup_error) = abandon_upload(
            metadata,
            segment_store.as_ref(),
            &candidate.repo_id,
            &reservation,
            cleanup_now_unix,
        )
        .await
        {
            tracing::warn!(
                error = %cleanup_error,
                segment_id = reservation.segment_id,
                "failed to discard Git compaction after lease loss"
            );
        }
        if let Err(error) = final_renewal {
            return Err(anyhow::anyhow!(
                "renewing Git compaction lease before publication: {}",
                error.message
            ));
        }
        return Ok(CompactionOutcome::Stale);
    }
    if let Err(error) = metadata
        .repositories()
        .mark_git_segment_upload_ready(
            &built.staged.segment,
            built.staged.encrypted_bytes,
            super::unix_now()?,
        )
        .await
    {
        let cleanup_now_unix = super::unix_now()?;
        if let Err(cleanup_error) = abandon_upload(
            metadata,
            segment_store.as_ref(),
            &candidate.repo_id,
            &reservation,
            cleanup_now_unix,
        )
        .await
        {
            tracing::warn!(
                error = %cleanup_error,
                segment_id = reservation.segment_id,
                "failed to discard Git compaction after ready transition failure"
            );
        }
        metadata
            .jobs()
            .fail_git_compaction_claim(&claim, &error.message, cleanup_now_unix)
            .await
            .map_err(|claim_error| anyhow::anyhow!(claim_error.message))?;
        return Err(anyhow::anyhow!(
            "marking Git compaction upload ready failed: {}",
            error.message
        ));
    }
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
            if applied {
                cleanup_retired_local_segments(
                    segment_store.as_ref(),
                    &candidate.repo_id,
                    &candidate.spans,
                )
                .await;
            } else if let Err(error) = delete_deleting_upload(
                metadata,
                segment_store.as_ref(),
                &candidate.repo_id,
                &reservation,
                super::unix_now()?,
            )
            .await
            {
                tracing::warn!(
                    error = %error,
                    segment_id = reservation.segment_id,
                    "failed to delete stale Git compaction upload"
                );
            }
            metadata
                .jobs()
                .continue_git_compaction_claim(&claim, super::unix_now()?)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?;
            let outcome = if applied { "applied" } else { "stale" };
            log_compaction_attempt(
                &claim,
                candidate,
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
            if let Err(cleanup_error) = abandon_upload(
                metadata,
                segment_store.as_ref(),
                &candidate.repo_id,
                &reservation,
                failure_now_unix,
            )
            .await
            {
                tracing::warn!(
                    error = %cleanup_error,
                    segment_id = reservation.segment_id,
                    "Git compaction publication failed and upload cleanup was deferred"
                );
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
        local_restore_count = metrics.pack.local_restore_count,
        remote_restore_count = metrics.pack.remote_restore_count,
        candidate_query_ms,
        init_ms = metrics.pack.init_ms,
        download_ms = metrics.pack.download_ms,
        index_ms = metrics.pack.index_ms,
        update_ref_ms = metrics.pack.update_ref_ms,
        connectivity_check_ms = metrics.pack.connectivity_check_ms,
        pack_ms = metrics.pack.pack_ms,
        pack_total_ms = metrics.pack.total_ms,
        local_write_and_fsync_ms = metrics
            .ingest
            .local_write_and_fsync
            .as_millis(),
        remote_multipart_upload_ms = metrics
            .ingest
            .remote_multipart_upload
            .as_millis(),
        encrypted_bytes = metrics.ingest.encrypted_bytes,
        uploaded_parts = metrics.ingest.uploaded_parts,
        ingest_chunk_bytes = metrics.ingest.chunk_bytes,
        ingest_channel_capacity = metrics.ingest.channel_capacity,
        persist_ms,
        total_ms,
        "Git compaction attempt completed"
    );
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

async fn abandon_upload(
    metadata: &MetadataStore,
    segment_store: &GitSegmentStore,
    repository_id: &str,
    reservation: &GitSegmentReservation,
    now_unix: u64,
) -> anyhow::Result<()> {
    let can_delete = metadata
        .repositories()
        .abandon_git_segment_upload(&reservation.segment_id, now_unix)
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let mut cleanup_error = None;
    let remote_deleted = if can_delete {
        match segment_store.cleanup_remote(&reservation.object_key).await {
            Ok(()) => true,
            Err(error) => {
                cleanup_error = Some(anyhow::Error::new(error));
                false
            }
        }
    } else {
        false
    };
    let local_deleted = match segment_store
        .cleanup_local(repository_id, &reservation.segment_id)
        .await
    {
        Ok(()) => true,
        Err(error) => {
            if cleanup_error.is_none() {
                cleanup_error = Some(anyhow::Error::new(error));
            }
            false
        }
    };
    if can_delete
        && remote_deleted
        && local_deleted
        && let Err(error) = metadata
            .repositories()
            .mark_git_segment_upload_deleted(&reservation.segment_id, now_unix)
            .await
        && cleanup_error.is_none()
    {
        cleanup_error = Some(anyhow::anyhow!(error.message));
    }
    if let Some(error) = cleanup_error {
        return Err(error);
    }
    Ok(())
}

async fn delete_deleting_upload(
    metadata: &MetadataStore,
    segment_store: &GitSegmentStore,
    repository_id: &str,
    reservation: &GitSegmentReservation,
    now_unix: u64,
) -> anyhow::Result<()> {
    let (remote_deleted, mut cleanup_error) =
        match segment_store.delete_remote(&reservation.object_key).await {
            Ok(()) => (true, None),
            Err(error) => (false, Some(anyhow::Error::new(error))),
        };
    let local_deleted = match segment_store
        .cleanup_local(repository_id, &reservation.segment_id)
        .await
    {
        Ok(()) => true,
        Err(error) => {
            if cleanup_error.is_none() {
                cleanup_error = Some(anyhow::Error::new(error));
            }
            false
        }
    };
    if remote_deleted
        && local_deleted
        && let Err(error) = metadata
            .repositories()
            .mark_git_segment_upload_deleted(&reservation.segment_id, now_unix)
            .await
        && cleanup_error.is_none()
    {
        cleanup_error = Some(anyhow::anyhow!(error.message));
    }
    cleanup_error.map_or(Ok(()), Err)
}

struct BuiltCompaction {
    replacement: GitPackSpan,
    staged: StagedGitSegment,
    metrics: CompactionMetrics,
}

struct CompactionMetrics {
    pack: CompactionPackMetrics,
    ingest: GitSegmentIngestTimings,
}

async fn cleanup_retired_local_segments(
    segment_store: &GitSegmentStore,
    repository_id: &str,
    spans: &[GitPackSpan],
) {
    for span in spans {
        if let Err(error) = segment_store
            .cleanup_local(repository_id, &span.segment.segment_id)
            .await
        {
            tracing::warn!(
                error = %error,
                segment_id = span.segment.segment_id,
                "failed to remove retired local Git segment"
            );
        }
    }
}

async fn build_compacted_span(
    segment_store: Arc<GitSegmentStore>,
    candidate: &scope_postgres::db::GitCompactionCandidate,
    reservation: GitSegmentReservation,
    storage_limits: GitStorageLimits,
    timeout: Duration,
    data_dir: std::path::PathBuf,
) -> Result<BuiltCompaction, CompactedPackFailure> {
    let pack = build_compacted_pack(
        segment_store,
        candidate,
        reservation,
        storage_limits,
        timeout,
        data_dir,
    )
    .await?;
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
        segment: pack.staged.segment.clone(),
    };
    replacement.geometric_tier = replacement
        .expected_geometric_tier()
        .map_err(|error| CompactedPackFailure::from(anyhow::Error::new(error)))?;
    Ok(BuiltCompaction {
        replacement,
        metrics: CompactionMetrics {
            pack: pack.metrics,
            ingest: pack.staged.timings.clone(),
        },
        staged: pack.staged,
    })
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
