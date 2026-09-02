use crate::{
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::{import::run_git, upload::truncated_git_stderr},
    state::AppState,
};
use scope_domain::repository::git::{GitHead, GitPackSpan, validate_git_pack_layout};
use scope_git_process::{ProcessLimits, run_with_stdin_reader};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

pub(crate) async fn restore_git_pack_spans(
    state: &AppState,
    repository_id: &str,
    head: &GitHead,
    pack_spans: &[GitPackSpan],
    repo_root: &Path,
) -> Result<(), ApiError> {
    let started_at = Instant::now();
    let total_pack_bytes = pack_spans
        .iter()
        .map(|span| span.segment.plaintext_bytes)
        .sum::<u64>();
    let result =
        restore_git_pack_spans_inner(state, repository_id, head, pack_spans, repo_root).await;
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

async fn restore_git_pack_spans_inner(
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
    let repo_root_for_cleanup = repo_root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        if repo_root_for_cleanup.exists() {
            fs::remove_dir_all(repo_root_for_cleanup).map_err(ApiError::internal)?;
        }
        Ok::<_, ApiError>(())
    })
    .await
    .map_err(|error| {
        ApiError::internal_message(format!("Git restore cleanup task failed: {error}"))
    })??;
    run_timed_git_restore_phase_async(
        repository_id,
        "init",
        None,
        vec![
            "init".to_string(),
            "--bare".to_string(),
            repo_root.to_string_lossy().into_owned(),
        ],
        "initializing Git snapshot repo",
    )
    .await?;
    for (index, span) in pack_spans.iter().enumerate() {
        index_git_pack(
            state,
            repo_root,
            repository_id,
            span,
            index + 1,
            pack_spans.len(),
        )
        .await?;
    }
    run_timed_git_restore_phase_async(
        repository_id,
        "update_ref",
        Some(repo_root.to_path_buf()),
        vec![
            "update-ref".to_string(),
            format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            head.head_oid.clone(),
        ],
        "restoring Git pack-layout head",
    )
    .await?;
    run_timed_git_restore_phase_async(
        repository_id,
        "fsck",
        Some(repo_root.to_path_buf()),
        vec![
            "fsck".to_string(),
            "--connectivity-only".to_string(),
            head.head_oid.clone(),
        ],
        "verifying restored Git pack layout",
    )
    .await?;
    run_timed_git_restore_phase_async(
        repository_id,
        "symbolic_ref",
        Some(repo_root.to_path_buf()),
        vec![
            "symbolic-ref".to_string(),
            "HEAD".to_string(),
            format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "setting restored Git snapshot head",
    )
    .await?;
    Ok(())
}

pub(crate) async fn index_git_pack(
    state: &AppState,
    repo_root: &Path,
    repository_id: &str,
    span: &GitPackSpan,
    span_index: usize,
    span_count: usize,
) -> Result<(), ApiError> {
    let temp_name = format!(
        "scope-segment-{}.pack.tmp",
        hex::encode(Sha256::digest(span.segment.segment_id.as_bytes()))
    );
    let temp_pack = repo_root.join(temp_name);
    let retrieval_started = Instant::now();
    let restore = async {
        let mut output = tokio::fs::File::create(&temp_pack)
            .await
            .map_err(scope_git_storage::GitStorageError::Local)?;
        let timings = state
            .git_segment_store
            .restore_to_prefer_local(repository_id, &span.segment, &mut output)
            .await?;
        output
            .sync_all()
            .await
            .map_err(scope_git_storage::GitStorageError::Local)?;
        Ok::<_, scope_git_storage::GitStorageError>(timings)
    }
    .await;
    let retrieval_elapsed = retrieval_started.elapsed();
    tracing::info!(
        phase = "verified",
        repository_id,
        segment_id = span.segment.segment_id,
        source = ?restore.as_ref().ok().map(|timings| timings.source),
        success = restore.is_ok(),
        duration_us = retrieval_elapsed.as_micros(),
        bytes = span.segment.plaintext_bytes,
        "Git segment restore telemetry"
    );
    if let Err(error) = restore {
        let _ = tokio::fs::remove_file(&temp_pack).await;
        return Err(ApiError::infrastructure_unavailable(error.to_string()));
    }
    let timeout = state.runtime_budgets.git_command_timeout();
    let repo_root = repo_root.to_path_buf();
    let repository_id = repository_id.to_string();
    let span = span.clone();
    let temp_pack_for_index = temp_pack.clone();
    let indexed = tokio::task::spawn_blocking(move || {
        index_restored_git_pack(
            &repo_root,
            &repository_id,
            &span,
            span_index,
            span_count,
            &temp_pack_for_index,
            timeout,
        )
    })
    .await;
    let _ = tokio::fs::remove_file(&temp_pack).await;
    indexed.map_err(|error| {
        ApiError::internal_message(format!("Git index-pack task failed: {error}"))
    })?
}

fn index_restored_git_pack(
    repo_root: &Path,
    repository_id: &str,
    span: &GitPackSpan,
    span_index: usize,
    span_count: usize,
    temp_pack: &Path,
    timeout: Duration,
) -> Result<(), ApiError> {
    let size_bytes = span.segment.plaintext_bytes;
    let started_at = Instant::now();
    let pack_file = fs::File::open(temp_pack).map_err(ApiError::internal)?;
    let output = run_with_stdin_reader(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo_root)
            .args(["index-pack", "--stdin"]),
        pack_file,
        ProcessLimits::new(timeout),
        "restoring Git pack",
    )
    .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()));
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
        object_sha256 = span.segment.sha256,
        success,
        "Git restore operation completed"
    );
    let output = output?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "restoring Git pack: {}",
            truncated_git_stderr(&output.stderr).trim()
        )));
    }
    Ok(())
}

pub(crate) async fn run_timed_git_restore_phase_async(
    repository_id: &str,
    operation: &'static str,
    repo_root: Option<PathBuf>,
    args: Vec<String>,
    context: &'static str,
) -> Result<(), ApiError> {
    let repository_id = repository_id.to_string();
    tokio::task::spawn_blocking(move || {
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        run_timed_git_restore_phase(
            &repository_id,
            operation,
            repo_root.as_deref(),
            &args,
            context,
        )
    })
    .await
    .map_err(|error| ApiError::internal_message(format!("Git restore task failed: {error}")))?
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
