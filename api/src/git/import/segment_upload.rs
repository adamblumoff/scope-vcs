use crate::{error::ApiError, state::AppState};
use scope_git_storage::{ENCODING_VERSION, GitSegmentReservation, StagedGitSegment};
use std::time::Duration;

pub(super) enum RemoteCleanup {
    Attempt,
    Deferred,
}

pub(crate) struct GitSegmentUploadHeartbeat {
    task: tokio::task::JoinHandle<()>,
}

impl GitSegmentUploadHeartbeat {
    pub(crate) fn start(state: &AppState, segment_id: String) -> Self {
        let metadata = state.metadata.clone();
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let now = match crate::persistence::unix_now() {
                    Ok(now) => now,
                    Err(error) => {
                        tracing::warn!(segment_id, error = %error.into_operator_diagnostic(), "Git segment upload heartbeat clock failed");
                        continue;
                    }
                };
                match metadata
                    .repositories()
                    .touch_git_segment_upload(&segment_id, now)
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(error) => {
                        tracing::warn!(segment_id, error = %error.message, "Git segment upload heartbeat failed")
                    }
                }
            }
        });
        Self { task }
    }
}

impl Drop for GitSegmentUploadHeartbeat {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) async fn begin_git_segment_upload(
    state: &AppState,
    repository_id: &str,
) -> Result<(GitSegmentReservation, GitSegmentUploadHeartbeat), ApiError> {
    let reservation = state
        .git_segment_store
        .reserve(repository_id)
        .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?;
    state
        .metadata
        .repositories()
        .begin_git_segment_upload(
            repository_id,
            &reservation.segment_id,
            &reservation.object_key,
            ENCODING_VERSION,
            crate::persistence::unix_now()?,
        )
        .await?;
    let heartbeat = GitSegmentUploadHeartbeat::start(state, reservation.segment_id.clone());
    Ok((reservation, heartbeat))
}

pub(crate) async fn best_effort_delete_staged_git_segment(
    state: &AppState,
    repository_id: &str,
    staged: &StagedGitSegment,
) {
    best_effort_delete_git_segment_identity(
        state,
        repository_id,
        &staged.segment.segment_id,
        &staged.object_key,
        RemoteCleanup::Attempt,
    )
    .await;
    if let Err(error) = state.git_segment_store.delete_local(staged).await {
        tracing::warn!(
            repository_id,
            segment_id = staged.segment.segment_id,
            error = %error,
            "failed to delete local Git segment"
        );
    }
}

pub(super) async fn best_effort_delete_git_segment_identity(
    state: &AppState,
    repository_id: &str,
    segment_id: &str,
    object_key: &str,
    remote_cleanup: RemoteCleanup,
) {
    let now = crate::persistence::unix_now().unwrap_or(0);
    let abandoned = match state
        .metadata
        .repositories()
        .abandon_git_segment_upload(segment_id, now)
        .await
    {
        Ok(abandoned) => abandoned,
        Err(error) => {
            tracing::warn!(repository_id, segment_id, error = %error.message, "failed to abandon Git segment");
            return;
        }
    };
    if !abandoned {
        tracing::warn!(
            repository_id,
            segment_id,
            "Git segment may already be published; leaving remote bytes untouched"
        );
        return;
    }
    if matches!(remote_cleanup, RemoteCleanup::Deferred) {
        tracing::warn!(
            repository_id,
            segment_id,
            "Git segment cleanup deferred after process timeout"
        );
        return;
    }
    if let Err(error) = state
        .git_segment_store
        .cleanup_remote_bounded(object_key)
        .await
    {
        tracing::warn!(repository_id, segment_id, error = %error, "failed to delete remote Git segment");
        return;
    }
    if let Err(error) = state
        .metadata
        .repositories()
        .mark_git_segment_upload_deleted(segment_id, crate::persistence::unix_now().unwrap_or(now))
        .await
    {
        tracing::warn!(repository_id, segment_id, error = %error.message, "failed to mark Git segment deleted");
    }
}
