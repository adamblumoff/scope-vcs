use crate::{
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::{import::run_git, upload::git_process_output_with_timeout},
    state::AppState,
};
use scope_domain::store::{GitHead, GitPackSpan, SourceBlob, validate_git_pack_layout};
use scope_object_store::source_blob_bytes;
use std::{fs, path::Path, process::Command, time::Instant};

pub(crate) fn restore_git_pack_spans(
    state: &AppState,
    repository_id: &str,
    head: &GitHead,
    pack_spans: &[GitPackSpan],
    repo_root: &Path,
) -> Result<(), ApiError> {
    let started_at = Instant::now();
    let total_pack_bytes = pack_spans
        .iter()
        .map(|span| span.object.size_bytes)
        .sum::<u64>();
    let result = restore_git_pack_spans_inner(state, repository_id, head, pack_spans, repo_root);
    tracing::info!(
        repository_id,
        operation = "restore_pack_layout",
        duration_ms = started_at.elapsed().as_millis(),
        requested_sequence = head.push_sequence,
        pack_span_count = pack_spans.len(),
        total_pack_bytes,
        success = result.is_ok(),
        "Git restore operation completed"
    );
    result
}

fn restore_git_pack_spans_inner(
    state: &AppState,
    repository_id: &str,
    head: &GitHead,
    pack_spans: &[GitPackSpan],
    repo_root: &Path,
) -> Result<(), ApiError> {
    validate_git_pack_layout(pack_spans)
        .map_err(|error| ApiError::internal_message(error.to_string()))?;
    let final_span = pack_spans
        .last()
        .ok_or_else(|| ApiError::internal_message("Git head has no physical pack spans"))?;
    if final_span.last_sequence != head.push_sequence || final_span.head_oid != head.head_oid {
        return Err(ApiError::internal_message(
            "Git pack layout frontier does not match the logical head",
        ));
    }
    if repo_root.exists() {
        fs::remove_dir_all(repo_root).map_err(ApiError::internal)?;
    }
    run_timed_git_restore_phase(
        repository_id,
        "init",
        None,
        &["init", "--bare", repo_root.to_string_lossy().as_ref()],
        "initializing Git snapshot repo",
    )?;
    for (index, span) in pack_spans.iter().enumerate() {
        index_git_pack(
            state,
            repo_root,
            repository_id,
            span,
            index + 1,
            pack_spans.len(),
        )?;
    }
    run_timed_git_restore_phase(
        repository_id,
        "update_ref",
        Some(repo_root),
        &[
            "update-ref",
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            &head.head_oid,
        ],
        "restoring Git pack-layout head",
    )?;
    run_timed_git_restore_phase(
        repository_id,
        "fsck",
        Some(repo_root),
        &["fsck", "--connectivity-only", &head.head_oid],
        "verifying restored Git pack layout",
    )?;
    run_timed_git_restore_phase(
        repository_id,
        "symbolic_ref",
        Some(repo_root),
        &[
            "symbolic-ref",
            "HEAD",
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "setting restored Git snapshot head",
    )?;
    Ok(())
}

pub(crate) fn index_git_pack(
    state: &AppState,
    repo_root: &Path,
    repository_id: &str,
    span: &GitPackSpan,
    span_index: usize,
    span_count: usize,
) -> Result<(), ApiError> {
    let bytes = restore_object_bytes(
        state,
        &span.object,
        repository_id,
        span,
        span_index,
        span_count,
    )?;
    let size_bytes = bytes.len();
    let started_at = Instant::now();
    let output = git_process_output_with_timeout(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo_root)
            .args(["index-pack", "--stdin"]),
        Some(bytes),
        state.runtime_budgets.git_command_timeout(),
    );
    let success = output.as_ref().is_ok_and(|output| output.status.success());
    let duration_ms = started_at.elapsed().as_millis();
    tracing::info!(
        repository_id,
        operation = "index_pack",
        duration_ms,
        repo_git_index_pack_ms = duration_ms,
        size_bytes,
        span_index,
        span_count,
        first_sequence = span.first_sequence,
        last_sequence = span.last_sequence,
        geometric_tier = span.geometric_tier,
        object_sha256 = span.object.sha256,
        success,
        "Git restore operation completed"
    );
    let output = output?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "restoring Git pack: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn restore_object_bytes(
    state: &AppState,
    blob: &SourceBlob,
    repository_id: &str,
    span: &GitPackSpan,
    span_index: usize,
    span_count: usize,
) -> Result<Vec<u8>, ApiError> {
    let started_at = Instant::now();
    let bytes = source_blob_bytes(state.object_store.as_ref(), blob).map_err(ApiError::from);
    let size_bytes = bytes.as_ref().map_or(blob.size_bytes, |bytes| {
        u64::try_from(bytes.len()).unwrap_or(u64::MAX)
    });
    let duration_ms = started_at.elapsed().as_millis();
    tracing::info!(
        repository_id,
        operation = "object_retrieval",
        object_kind = "pack",
        duration_ms,
        repo_git_object_retrieval_ms = duration_ms,
        size_bytes,
        span_index,
        span_count,
        first_sequence = span.first_sequence,
        last_sequence = span.last_sequence,
        geometric_tier = span.geometric_tier,
        object_sha256 = blob.sha256,
        content_ref = ?blob.content_ref,
        success = bytes.is_ok(),
        "Git restore operation completed"
    );
    bytes
}

pub(crate) fn run_timed_git_restore_phase(
    repository_id: &str,
    operation: &'static str,
    repo_root: Option<&Path>,
    args: &[&str],
    context: &'static str,
) -> Result<(), ApiError> {
    let started_at = Instant::now();
    let result = run_git(repo_root, args, context);
    tracing::info!(
        repository_id,
        operation,
        duration_ms = started_at.elapsed().as_millis(),
        success = result.is_ok(),
        "Git restore operation completed"
    );
    result
}
