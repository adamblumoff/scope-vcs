use super::northflank::{NorthflankClient, StartError};
use crate::settings::CloudExecutionSettings;
use scope_domain::runs::run::AttemptConclusion;
use scope_postgres::db::MetadataStore;
use sha2::{Digest as _, Sha256};
use std::time::Duration;

const DISPATCH_LEASE: Duration = Duration::from_secs(15 * 60);

pub(crate) struct CloudExecutionCoordinator {
    metadata: MetadataStore,
    northflank: NorthflankClient,
    settings: CloudExecutionSettings,
}

impl CloudExecutionCoordinator {
    pub(crate) fn new(
        metadata: MetadataStore,
        settings: CloudExecutionSettings,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            metadata,
            northflank: NorthflankClient::new(settings.clone())?,
            settings,
        })
    }

    pub(crate) async fn dispatch_available(&self, now_unix: u64) -> anyhow::Result<usize> {
        let active = self
            .metadata
            .runs()
            .active_cloud_attempt_count()
            .await
            .map_err(db_error)? as usize;
        let available = self.settings.max_concurrency.saturating_sub(active);
        let mut dispatched = 0;
        while dispatched < available {
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
                Err(error) if error.kind == scope_postgres::error::PostgresErrorKind::Conflict => {
                    continue;
                }
                Err(error) => return Err(db_error(error)),
            };
            let definition = claim
                .workflow_revision
                .definition()
                .job(&claim.job.key)
                .ok_or_else(|| anyhow::anyhow!("dispatched workflow job definition is missing"))?;
            match self
                .northflank
                .start(
                    definition.container().image(),
                    &attempt_id,
                    &bootstrap_token,
                )
                .await
            {
                Ok(external_run_id) => {
                    self.metadata
                        .runs()
                        .record_external_run_id(&attempt_id, &external_run_id)
                        .await
                        .map_err(db_error)?;
                    tracing::info!(attempt_id, external_run_id, run_id = %claim.run.id, job = %claim.job.key.as_str(), "dispatched cloud run");
                }
                Err(StartError::Rejected(error)) => {
                    tracing::error!(attempt_id, error = %error, "Northflank rejected cloud run");
                    self.metadata
                        .runs()
                        .complete_attempt(
                            &attempt_id,
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
                }
                Err(StartError::Ambiguous(error)) => {
                    tracing::warn!(attempt_id, error = %error, "Northflank dispatch outcome is ambiguous; lease recovery owns resolution");
                }
            }
            dispatched += 1;
        }
        Ok(dispatched)
    }

    pub(crate) async fn abort_canceled(&self, now_unix: u64) -> anyhow::Result<usize> {
        let attempts = self
            .metadata
            .runs()
            .claim_cloud_attempt_aborts(now_unix, self.settings.max_concurrency as u64)
            .await
            .map_err(db_error)?;
        let mut aborted = 0;
        for attempt in attempts {
            match self.northflank.abort(&attempt.external_run_id).await {
                Ok(()) => {
                    self.metadata
                        .runs()
                        .confirm_provider_cancellation(&attempt.attempt_id, now_unix)
                        .await
                        .map_err(db_error)?;
                    aborted += 1;
                    tracing::info!(attempt_id = %attempt.attempt_id, external_run_id = %attempt.external_run_id, "aborted canceled cloud run");
                }
                Err(error) => {
                    self.metadata
                        .runs()
                        .release_cloud_attempt_abort(&attempt.attempt_id)
                        .await
                        .map_err(db_error)?;
                    tracing::warn!(attempt_id = %attempt.attempt_id, error = %error, "failed to abort canceled cloud run; will retry");
                }
            }
        }
        Ok(aborted)
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
mod tests {
    use super::*;

    #[test]
    fn bootstrap_tokens_use_the_runtime_auth_prefix() {
        let token = random_token("scope_bootstrap_").unwrap();
        assert!(token.starts_with("scope_bootstrap_"));
        assert_eq!(token.len(), "scope_bootstrap_".len() + 64);
    }
}
