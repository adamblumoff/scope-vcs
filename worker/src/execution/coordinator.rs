use super::ecs::{EcsClient, StartError};
use super::provisioning::Provisioning;
use crate::settings::CloudExecutionSettings;
use anyhow::Context as _;
use scope_domain::runs::{attempt::MAX_RUN_ATTEMPT_AGE_SECONDS, step::AttemptConclusion};
use scope_postgres::db::MetadataStore;
use sha2::{Digest as _, Sha256};
use std::time::Duration;

const DISPATCH_LEASE: Duration = Duration::from_secs(15 * 60);

#[derive(Clone)]
pub(crate) struct CloudExecutionCoordinator {
    metadata: MetadataStore,
    ecs: EcsClient,
    origin_id: String,
    settings: CloudExecutionSettings,
}

impl CloudExecutionCoordinator {
    pub(crate) async fn new(
        metadata: MetadataStore,
        settings: CloudExecutionSettings,
        origin_id: String,
    ) -> Self {
        Self {
            metadata,
            ecs: EcsClient::new(settings.clone()).await,
            origin_id,
            settings,
        }
    }

    pub(crate) async fn dispatch_available(&self, now_unix: u64) -> anyhow::Result<usize> {
        let active = self
            .metadata
            .runs()
            .active_cloud_attempt_count()
            .await
            .map_err(db_error)? as usize;
        let available = self.settings.max_concurrency.saturating_sub(active);
        let mut starts = Provisioning::new(self.settings.max_concurrency);
        let dispatch_result = async {
            let mut dispatched = 0;
            while dispatched < available {
                starts.wait_for_slot().await?;
                let Some(offer) = self
                    .metadata
                    .runs()
                    .next_dispatchable_job()
                    .await
                    .map_err(db_error)?
                else {
                    break;
                };
                let attempt_id = random_id("attempt")?;
                let bootstrap_token = random_token("scope_bootstrap_")?;
                let bootstrap_hash = hex::encode(Sha256::digest(bootstrap_token.as_bytes()));
                let claim = match self
                    .metadata
                    .runs()
                    .dispatch_job(
                        &offer.run.id,
                        offer.job.key.as_str(),
                        &attempt_id,
                        &bootstrap_hash,
                        &self.settings.runtime_version,
                        now_unix,
                        now_unix + DISPATCH_LEASE.as_secs(),
                    )
                    .await
                {
                    Ok(claim) => claim,
                    Err(error)
                        if error.kind == scope_postgres::error::PostgresErrorKind::Conflict =>
                    {
                        continue;
                    }
                    Err(error) => return Err(db_error(error)),
                };
                self.publish_status_change(&claim).await;
                let execution = self.clone();
                starts.spawn(async move {
                    execution
                        .provision(claim, bootstrap_token, bootstrap_hash, now_unix)
                        .await
                });
                dispatched += 1;
            }
            Ok(dispatched)
        }
        .await;
        // A failed admission or launch must not cancel other already-reserved starts.
        let provision_result = starts.finish().await;
        let dispatched = dispatch_result?;
        provision_result?;
        Ok(dispatched)
    }

    async fn provision(
        &self,
        claim: scope_postgres::db::DispatchClaim,
        bootstrap_token: String,
        bootstrap_hash: String,
        now_unix: u64,
    ) -> anyhow::Result<()> {
        let attempt_id = &claim.attempt.id;
        let definition = claim
            .workflow_revision
            .definition()
            .job(&claim.job.key)
            .ok_or_else(|| anyhow::anyhow!("dispatched workflow job definition is missing"))?;
        match self
            .ecs
            .start(
                definition.container().image(),
                attempt_id,
                &bootstrap_token,
                now_unix + MAX_RUN_ATTEMPT_AGE_SECONDS,
            )
            .await
        {
            Ok(external_run_id) => {
                self.metadata
                    .runs()
                    .record_external_run_id(attempt_id, &external_run_id)
                    .await
                    .map_err(db_error)?;
                self.publish_status_change(&claim).await;
                tracing::info!(attempt_id = %attempt_id, external_run_id, run_id = %claim.run.id, job = %claim.job.key.as_str(), "dispatched cloud run");
            }
            Err(StartError::Rejected(error)) => {
                tracing::error!(attempt_id = %attempt_id, error = %error, "ECS rejected cloud run");
                let claim = self
                    .metadata
                    .runs()
                    .complete_attempt(
                        attempt_id,
                        &bootstrap_hash,
                        AttemptConclusion::SetupFailed {
                            exit_code: 69,
                            message: format!("provider rejected dispatch: {error}")
                                .chars()
                                .take(2048)
                                .collect(),
                        },
                        false,
                        now_unix,
                    )
                    .await
                    .map_err(db_error)?;
                self.metadata
                    .runs()
                    .complete_cloud_task_absence(attempt_id, now_unix)
                    .await
                    .map_err(db_error)?;
                self.publish_status_change(&claim).await;
            }
            Err(StartError::Ambiguous(error)) => {
                tracing::warn!(attempt_id = %attempt_id, error = %error, "ECS dispatch outcome is ambiguous; lease recovery and task cleanup own resolution");
            }
        }
        Ok(())
    }

    pub(crate) async fn abort_canceled(&self, now_unix: u64) -> anyhow::Result<usize> {
        let attempts = self
            .metadata
            .runs()
            .claim_cloud_attempt_aborts(now_unix, self.settings.max_concurrency as u64)
            .await
            .map_err(db_error)?;
        let mut tasks = tokio::task::JoinSet::new();
        for attempt in attempts {
            let metadata = self.metadata.clone();
            let ecs = self.ecs.clone();
            tasks.spawn(
                async move { abort_canceled_attempt(metadata, ecs, attempt, now_unix).await },
            );
        }
        let mut aborted = 0;
        while let Some(result) = tasks.join_next().await {
            if let Some(claim) = result.context("cloud cancellation task panicked")?? {
                self.publish_status_change(&claim).await;
                aborted += 1;
            }
        }
        Ok(aborted)
    }

    pub(crate) async fn cleanup_terminal(&self, now_unix: u64) -> anyhow::Result<usize> {
        let tasks = self
            .metadata
            .runs()
            .claim_terminal_cloud_task_stops(now_unix, self.settings.max_concurrency as u64)
            .await
            .map_err(db_error)?;
        let mut reconciliations = tokio::task::JoinSet::new();
        for task in tasks {
            let metadata = self.metadata.clone();
            let ecs = self.ecs.clone();
            reconciliations
                .spawn(async move { cleanup_terminal_task(metadata, ecs, task, now_unix).await });
        }
        let mut stopped = 0;
        while let Some(result) = reconciliations.join_next().await {
            if result.context("terminal cloud cleanup task panicked")?? {
                stopped += 1;
            }
        }
        Ok(stopped)
    }

    async fn publish_status_change(&self, claim: &scope_postgres::db::DispatchClaim) {
        crate::run_events::publish_run_change(
            &self.metadata,
            &self.origin_id,
            claim.run.workflow.repository_id(),
            &claim.run.id,
            scope_api_contract::RunChangeKind::StatusChanged,
        )
        .await;
    }
}

async fn abort_canceled_attempt(
    metadata: MetadataStore,
    ecs: EcsClient,
    attempt: scope_postgres::db::CloudTaskStop,
    now_unix: u64,
) -> anyhow::Result<Option<scope_postgres::db::DispatchClaim>> {
    match ecs
        .stop_terminal_task(&attempt.attempt_id, attempt.external_run_id.as_deref())
        .await
    {
        Ok(()) => {
            let claim = metadata
                .runs()
                .confirm_provider_cancellation(&attempt.attempt_id, now_unix)
                .await
                .map_err(db_error)?;
            metadata
                .runs()
                .complete_cloud_task_stop(&attempt.attempt_id, now_unix)
                .await
                .map_err(db_error)?;
            tracing::info!(attempt_id = %attempt.attempt_id, external_run_id = ?attempt.external_run_id, "aborted canceled cloud run");
            Ok(Some(claim))
        }
        Err(error) => {
            metadata
                .runs()
                .release_cloud_task_stop_claim(&attempt.attempt_id)
                .await
                .map_err(db_error)?;
            tracing::warn!(attempt_id = %attempt.attempt_id, error = %error, "failed to abort canceled cloud run; will retry");
            Ok(None)
        }
    }
}

async fn cleanup_terminal_task(
    metadata: MetadataStore,
    ecs: EcsClient,
    task: scope_postgres::db::CloudTaskStop,
    now_unix: u64,
) -> anyhow::Result<bool> {
    match ecs
        .stop_terminal_task(&task.attempt_id, task.external_run_id.as_deref())
        .await
    {
        Ok(()) => {
            metadata
                .runs()
                .complete_cloud_task_stop(&task.attempt_id, now_unix)
                .await
                .map_err(db_error)?;
            tracing::info!(attempt_id = %task.attempt_id, "reconciled terminal cloud task");
            Ok(true)
        }
        Err(error) => {
            metadata
                .runs()
                .release_cloud_task_stop_claim(&task.attempt_id)
                .await
                .map_err(db_error)?;
            tracing::warn!(attempt_id = %task.attempt_id, error = %error, "failed to reconcile terminal cloud task; will retry");
            Ok(false)
        }
    }
}

fn random_id(prefix: &str) -> anyhow::Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(format!("{prefix}_{}", hex::encode(bytes)))
}

fn random_token(prefix: &str) -> anyhow::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(format!("{prefix}{}", hex::encode(bytes)))
}

fn db_error(error: scope_postgres::error::PostgresError) -> anyhow::Error {
    anyhow::anyhow!(error.message)
}

#[cfg(test)]
mod tests;
