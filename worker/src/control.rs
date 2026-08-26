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
        if let Some(execution) = &execution {
            if let Err(error) = execution.cleanup_terminal(super::unix_now()?).await {
                tracing::error!(error = %error, "terminal cloud task cleanup failed");
            }
            if let Err(error) = execution.abort_canceled(super::unix_now()?).await {
                tracing::error!(error = %error, "cloud run cancellation reconciliation failed");
            }
            match execution.dispatch_available(super::unix_now()?).await {
                Ok(dispatched) if dispatched > 0 => {
                    tracing::info!(dispatched, "processed cloud run dispatches");
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::error!(error = %error, "cloud run dispatch failed; continuing control loop");
                }
            }
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
