use crate::{
    execution::CloudExecutionCoordinator,
    health::WorkerHealth,
    settings::{WorkerRole, WorkerSettings},
};
use scope_postgres::db::MetadataStore;

pub(crate) async fn run(
    metadata: MetadataStore,
    settings: WorkerSettings,
    health: WorkerHealth,
) -> anyhow::Result<()> {
    let execution = match settings.execution.clone() {
        Some(cloud) => Some(
            CloudExecutionCoordinator::new(metadata.clone(), cloud, settings.worker_id.clone())
                .await,
        ),
        None => None,
    };
    let mut cloud_reconciliation = None;
    loop {
        if !super::schema_ready_or_wait(&metadata, &health).await {
            return Ok(());
        }
        let summary = match metadata
            .jobs()
            .run_ready_outbox_jobs(
                &settings.worker_id,
                settings.batch_size,
                &|| super::unix_now().map_err(|error| error.to_string()),
                &super::generate_persistence_id,
            )
            .await
        {
            Ok(summary) => summary,
            Err(error) => {
                health.mark_schema_waiting();
                tracing::error!(error = %error.message, "control work failed; retrying");
                if super::wait_or_shutdown(settings.poll_interval).await {
                    return Ok(());
                }
                continue;
            }
        };
        if summary.claimed > 0 {
            tracing::info!(
                claimed = summary.claimed,
                completed = summary.completed,
                failed = summary.failed,
                "processed outbox jobs"
            );
        }
        for run in &summary.created_runs {
            crate::run_events::publish_run_change(
                &metadata,
                &settings.worker_id,
                &run.repo_id,
                &run.run_id,
                scope_api_contract::RunChangeKind::Created,
            )
            .await;
        }
        if cloud_reconciliation
            .as_ref()
            .is_some_and(tokio::task::JoinHandle::is_finished)
        {
            let completed = cloud_reconciliation
                .take()
                .expect("a finished reconciliation handle is present");
            if let Err(error) = completed.await {
                tracing::error!(error = %error, "cloud reconciliation task panicked");
            }
        }
        if cloud_reconciliation.is_none()
            && let Some(execution) = execution.clone()
        {
            cloud_reconciliation = Some(tokio::spawn(reconcile_cloud_execution(execution)));
        }
        health.mark_poll_succeeded(WorkerRole::Control, super::unix_now()?);
        if summary.claimed >= settings.batch_size {
            continue;
        }
        if super::wait_or_shutdown(settings.poll_interval).await {
            return Ok(());
        }
    }
}

async fn reconcile_cloud_execution(execution: CloudExecutionCoordinator) {
    match super::unix_now() {
        Ok(now_unix) => {
            if let Err(error) = execution.cleanup_terminal(now_unix).await {
                tracing::error!(error = %error, "terminal cloud task cleanup failed");
            }
        }
        Err(error) => tracing::error!(error = %error, "cloud cleanup clock failed"),
    }
    match super::unix_now() {
        Ok(now_unix) => {
            if let Err(error) = execution.abort_canceled(now_unix).await {
                tracing::error!(error = %error, "cloud run cancellation reconciliation failed");
            }
        }
        Err(error) => tracing::error!(error = %error, "cloud cancellation clock failed"),
    }
    match super::unix_now() {
        Ok(now_unix) => match execution.dispatch_available(now_unix).await {
            Ok(dispatched) if dispatched > 0 => {
                tracing::info!(dispatched, "processed cloud run dispatches");
            }
            Ok(_) => {}
            Err(error) => {
                tracing::error!(error = %error, "cloud run dispatch failed; continuing control loop");
            }
        },
        Err(error) => tracing::error!(error = %error, "cloud dispatch clock failed"),
    }
}
