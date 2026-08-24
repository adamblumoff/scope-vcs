use scope_api_contract::{
    RepositoryRunAttemptResponse, RepositoryRunCacheColdReason, RepositoryRunCacheFinalState,
    RepositoryRunCachePreparation, RepositoryRunCacheResponse, RepositoryRunDetailResponse,
    RepositoryRunJobState, RepositoryRunState,
};

pub(super) fn detail_lines(detail: &RepositoryRunDetailResponse) -> Vec<String> {
    let mut lines = vec![format!(
        "Run {} · {}",
        run_state_label(detail.run.state),
        short_oid(&detail.run.git_oid),
    )];
    lines.push("Jobs:".to_string());
    for job_detail in &detail.jobs {
        lines.push(format!(
            "  {} · {}",
            job_detail.job.key,
            job_state_label(job_detail.job.state),
        ));
        for attempt in &job_detail.attempts {
            lines.push(format!(
                "    {} · {} · {:?}",
                attempt.id,
                attempt_state_label(attempt),
                attempt.execution_provider,
            ));
            lines.extend(environment_lines(
                Some(job_detail.job.pinned_container_image.as_str()),
                attempt,
            ));
        }
    }
    lines
}

fn environment_lines(
    pinned_image: Option<&str>,
    attempt: &RepositoryRunAttemptResponse,
) -> Vec<String> {
    if attempt.caches.is_empty() && pinned_image.is_none() {
        return Vec::new();
    }
    let mut warm = 0;
    let mut cold = 0;
    let mut unavailable = 0;
    let mut prepare_ms = 0;
    for cache in &attempt.caches {
        match cache.observation.as_ref().map(|fact| fact.preparation) {
            Some(RepositoryRunCachePreparation::Warm) => warm += 1,
            Some(RepositoryRunCachePreparation::Cold { .. }) => cold += 1,
            None => unavailable += 1,
        }
        prepare_ms = prepare_ms.max(
            cache
                .observation
                .as_ref()
                .map(|fact| fact.prepare_ms)
                .unwrap_or_default(),
        );
    }
    let mut summary = Vec::new();
    if warm > 0 {
        summary.push(format!("{warm} warm"));
    }
    if cold > 0 {
        summary.push(format!("{cold} cold"));
    }
    if unavailable > 0 {
        summary.push(format!("{unavailable} not reported"));
    }
    if warm + cold > 0 {
        summary.push(format!("prepared {}", duration_label(prepare_ms)));
    }
    if let Some(image) = pinned_image {
        summary.push(image_label(image));
    }
    let mut lines = vec![format!("      Environment · {}", summary.join(" · "))];
    lines.extend(attempt.caches.iter().map(cache_line));
    lines
}

fn cache_line(cache: &RepositoryRunCacheResponse) -> String {
    let Some(observation) = &cache.observation else {
        return format!("        {} · not reported", cache.name);
    };
    let preparation = match observation.preparation {
        RepositoryRunCachePreparation::Warm => "warm".to_string(),
        RepositoryRunCachePreparation::Cold { reason } => {
            format!("cold · {}", cold_reason_label(reason))
        }
    };
    let finalization = finalization_label(observation.final_state, observation.finalize_ms);
    format!(
        "        {} · {} · {} · prepared {}",
        cache.name,
        preparation,
        finalization,
        duration_label(observation.prepare_ms),
    )
}

fn finalization_label(state: RepositoryRunCacheFinalState, finalize_ms: Option<u64>) -> String {
    let state = match state {
        RepositoryRunCacheFinalState::Pending => "pending",
        RepositoryRunCacheFinalState::Ready => "ready",
        RepositoryRunCacheFinalState::Evicted => "evicted",
    };
    finalize_ms.map_or_else(
        || state.to_string(),
        |milliseconds| format!("{state} · finalized {}", duration_label(milliseconds)),
    )
}

fn cold_reason_label(reason: RepositoryRunCacheColdReason) -> &'static str {
    match reason {
        RepositoryRunCacheColdReason::MetadataMissing => "no reusable entry for this identity",
        RepositoryRunCacheColdReason::MetadataInvalid => "cache metadata invalid",
        RepositoryRunCacheColdReason::MetadataNotReady => "cached volume not ready",
        RepositoryRunCacheColdReason::VolumeMissing => "cached volume missing",
        RepositoryRunCacheColdReason::VolumeInvalid => "cached volume invalid",
        RepositoryRunCacheColdReason::BackingDirectoryMissing => "cache backing directory missing",
    }
}

fn duration_label(milliseconds: u64) -> String {
    if milliseconds < 1_000 {
        return format!("{milliseconds}ms");
    }
    let tenths = milliseconds.saturating_add(50) / 100;
    format!("{}.{:01}s", tenths / 10, tenths % 10)
}

fn image_label(image: &str) -> String {
    let digest = image
        .rsplit_once("@sha256:")
        .map(|(_, digest)| digest)
        .unwrap_or(image);
    format!("image sha256:{}", digest.get(..12).unwrap_or(digest))
}

fn short_oid(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

fn run_state_label(state: RepositoryRunState) -> &'static str {
    match state {
        RepositoryRunState::Queued => "queued",
        RepositoryRunState::Dispatching => "dispatching",
        RepositoryRunState::Running => "running",
        RepositoryRunState::Succeeded => "succeeded",
        RepositoryRunState::Failed => "failed",
        RepositoryRunState::Canceled => "canceled",
        RepositoryRunState::Lost => "lost",
    }
}

fn job_state_label(state: RepositoryRunJobState) -> &'static str {
    match state {
        RepositoryRunJobState::Blocked => "blocked",
        RepositoryRunJobState::Queued => "queued",
        RepositoryRunJobState::Dispatching => "dispatching",
        RepositoryRunJobState::Running => "running",
        RepositoryRunJobState::Succeeded => "succeeded",
        RepositoryRunJobState::Failed => "failed",
        RepositoryRunJobState::Skipped => "skipped",
        RepositoryRunJobState::Canceled => "canceled",
        RepositoryRunJobState::Lost => "lost",
    }
}

fn attempt_state_label(attempt: &RepositoryRunAttemptResponse) -> &'static str {
    match attempt.state {
        scope_api_contract::RepositoryRunAttemptState::Dispatching => "dispatching",
        scope_api_contract::RepositoryRunAttemptState::Running => "running",
        scope_api_contract::RepositoryRunAttemptState::Succeeded => "succeeded",
        scope_api_contract::RepositoryRunAttemptState::Failed => "failed",
        scope_api_contract::RepositoryRunAttemptState::Canceled => "canceled",
        scope_api_contract::RepositoryRunAttemptState::Lost => "lost",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_api_contract::{
        RepositoryRunAttemptState, RepositoryRunCacheObservationResponse,
        RepositoryRunStepResponse, RepositoryRunSummaryResponse,
    };

    #[test]
    fn detail_output_distinguishes_cold_from_missing_reports() {
        let detail = RepositoryRunDetailResponse {
            run: RepositoryRunSummaryResponse {
                id: "run-1".to_string(),
                workflow_name: "checks".to_string(),
                git_oid: "1234567890".to_string(),
                trigger: scope_api_contract::RepositoryRunTrigger::PushMain,
                state: RepositoryRunState::Succeeded,
                cancellation_requested: false,
                created_at_unix: 1,
                updated_at_unix: 2,
                completed_at_unix: Some(2),
                can_cancel: false,
                can_retry: true,
            },
            jobs: vec![scope_api_contract::RepositoryRunJobDetailResponse {
                job: scope_api_contract::RepositoryRunJobResponse {
                    key: "backend".to_string(),
                    needs: vec![],
                    pinned_container_image: format!("registry/scope@sha256:{}", "a".repeat(64),),
                    state: RepositoryRunJobState::Succeeded,
                    created_at_unix: 1,
                    updated_at_unix: 2,
                    completed_at_unix: Some(2),
                },
                attempts: vec![RepositoryRunAttemptResponse {
                    id: "attempt-1".to_string(),
                    number: 1,
                    execution_provider: scope_api_contract::RepositoryExecutionProvider::Northflank,
                    external_run_id: Some("nf-run-1".to_string()),
                    runtime_version: "0.1.0".to_string(),
                    state: RepositoryRunAttemptState::Succeeded,
                    created_at_unix: 1,
                    started_at_unix: Some(1),
                    completed_at_unix: Some(2),
                    terminal_reason: None,
                    caches: vec![
                        RepositoryRunCacheResponse {
                            name: "cargo".to_string(),
                            path: "/cache/cargo".to_string(),
                            observation: Some(RepositoryRunCacheObservationResponse {
                                workflow_path: "/.scope/runs/checks.yml".to_string(),
                                job_key: "backend".to_string(),
                                identity_digest: "b".repeat(64),
                                preparation: RepositoryRunCachePreparation::Cold {
                                    reason: RepositoryRunCacheColdReason::MetadataMissing,
                                },
                                prepare_ms: 12,
                                final_state: RepositoryRunCacheFinalState::Ready,
                                finalize_ms: Some(8),
                            }),
                        },
                        RepositoryRunCacheResponse {
                            name: "target".to_string(),
                            path: "/cache/target".to_string(),
                            observation: None,
                        },
                        RepositoryRunCacheResponse {
                            name: "rustup".to_string(),
                            path: "/cache/rustup".to_string(),
                            observation: Some(RepositoryRunCacheObservationResponse {
                                workflow_path: "/.scope/runs/checks.yml".to_string(),
                                job_key: "backend".to_string(),
                                identity_digest: "c".repeat(64),
                                preparation: RepositoryRunCachePreparation::Warm,
                                prepare_ms: 50,
                                final_state: RepositoryRunCacheFinalState::Ready,
                                finalize_ms: Some(4),
                            }),
                        },
                    ],
                    steps: Vec::<RepositoryRunStepResponse>::new(),
                }],
            }],
        };

        let output = detail_lines(&detail).join("\n");
        assert!(output.contains("1 warm · 1 cold · 1 not reported · prepared 50ms"));
        assert!(output.contains("no reusable entry for this identity"));
        assert!(output.contains("target · not reported"));
        assert!(!output.contains("identity changed"));
    }
}
