use super::{
    content_cleanup::best_effort_cleanup_rollback_source_blobs,
    run_inspection::{InspectedRun, require_run_access},
};
use crate::{
    error::ApiError, git::run_source::inspect_manual_run_bundle, persistence::unix_now,
    state::AppState,
};
use scope_api_contract::RunChangeKind;
use scope_domain::runs::{manual::ManualRunRequest, run::Run, source::RunSource};
use scope_object_store::{ContentObjectKind, content_object_for_bytes, object_key};

pub(crate) struct ManualRunCommand {
    pub(crate) request: ManualRunRequest,
    pub(crate) bundle: Vec<u8>,
}

pub(crate) async fn resolve_manual_run(
    state: &AppState,
    request: &ManualRunRequest,
) -> Result<Option<InspectedRun>, ApiError> {
    match state
        .metadata
        .runs()
        .enqueue_known_manual_run(request, unix_now()?)
        .await?
    {
        Some(enqueued) => finish_enqueued_run(state, enqueued).await.map(Some),
        None => Ok(None),
    }
}

pub(crate) async fn create_manual_run(
    state: &AppState,
    command: ManualRunCommand,
) -> Result<InspectedRun, ApiError> {
    let inspect_root = state.data_dir.join("run-bundle-inspection");
    let bundle = command.bundle;
    let git_oid = command.request.git_oid().to_string();
    let workflow_name = command.request.workflow_name().to_string();
    let inspected = tokio::task::spawn_blocking(move || {
        inspect_manual_run_bundle(&inspect_root, &bundle, &git_oid, &workflow_name)
            .map(|workflow| (bundle, git_oid, workflow))
    })
    .await
    .map_err(|error| {
        ApiError::internal_message(format!("run bundle inspection failed: {error}"))
    })??;
    let (bundle, git_oid, parsed_workflow) = inspected;
    let revision = parsed_workflow
        .into_revision(command.request.repository_id())
        .map_err(ApiError::bad_request)?;
    let mut stored = content_object_for_bytes(ContentObjectKind::GitBundle, &bundle);
    stored.git_oid = git_oid;
    let source_cleanup = stored.clone();
    let run = command.request.create_run(
        &revision,
        RunSource::ephemeral_git_bundle(stored)?,
        unix_now()?,
    )?;
    let fence = state
        .metadata
        .acquire_content_ref_fence(std::slice::from_ref(&source_cleanup.content_ref))
        .await?;
    state
        .object_store
        .put(&object_key(&source_cleanup), bundle)?;
    let enqueued = match state.metadata.runs().enqueue_run(run, revision).await {
        Ok(enqueued) => enqueued,
        Err(error) => {
            best_effort_cleanup_rollback_source_blobs(state, &[source_cleanup]).await;
            fence.release().await;
            return Err(error.into());
        }
    };
    fence.release().await;
    finish_enqueued_run(state, enqueued).await
}

async fn finish_enqueued_run(
    state: &AppState,
    enqueued: scope_postgres::db::EnqueueRunResult,
) -> Result<InspectedRun, ApiError> {
    let run = enqueued.run;
    if enqueued.inserted {
        state
            .publish_run_change(
                run.workflow.repository_id(),
                run.id.clone(),
                RunChangeKind::Created,
            )
            .await;
    }
    let logs_truncated = !enqueued.inserted
        && state
            .metadata
            .runs()
            .run_has_truncated_logs(&run.id)
            .await?;
    let jobs = state.metadata.runs().run_jobs(&run.id).await?;
    Ok(InspectedRun {
        run,
        jobs,
        logs_truncated,
    })
}

pub(crate) async fn cancel_run(
    state: &AppState,
    user_id: &str,
    owner: &str,
    repo_name: &str,
    run_id: &str,
) -> Result<InspectedRun, ApiError> {
    require_run_access(state, user_id, owner, repo_name, run_id).await?;
    let run = state
        .metadata
        .runs()
        .request_run_cancellation(run_id, unix_now()?)
        .await?;
    finish_run_control(state, run).await
}

pub(crate) async fn retry_run(
    state: &AppState,
    user_id: &str,
    owner: &str,
    repo_name: &str,
    run_id: &str,
) -> Result<InspectedRun, ApiError> {
    require_run_access(state, user_id, owner, repo_name, run_id).await?;
    let run = state.metadata.runs().retry_run(run_id, unix_now()?).await?;
    finish_run_control(state, run).await
}

async fn finish_run_control(state: &AppState, run: Run) -> Result<InspectedRun, ApiError> {
    state
        .publish_run_change(
            run.workflow.repository_id(),
            run.id.clone(),
            RunChangeKind::StatusChanged,
        )
        .await;
    let logs_truncated = state
        .metadata
        .runs()
        .run_has_truncated_logs(&run.id)
        .await?;
    let jobs = state.metadata.runs().run_jobs(&run.id).await?;
    Ok(InspectedRun {
        run,
        jobs,
        logs_truncated,
    })
}
