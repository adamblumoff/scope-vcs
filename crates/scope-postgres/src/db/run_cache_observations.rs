use super::{RunStore, entities};
use crate::error::PostgresError;
use scope_domain::runs::{
    cache::{
        AttemptCacheObservation, AttemptCachePreparationTiming, AttemptCacheSetupObservation,
        CacheFinalState, CachePreparation,
    },
    workflow::WorkflowPath,
};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, QuerySelect, TransactionTrait};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptCachePreparationCommand {
    pub cache_name: String,
    pub identity_digest: String,
    pub preparation: CachePreparation,
    pub key_ms: u64,
    pub metadata_ms: u64,
    pub size_bytes: u64,
    pub download_verify_ms: u64,
    pub sync_ms: u64,
    pub extraction_ms: u64,
    pub prepare_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptCacheFinalizationCommand {
    pub identity_digest: String,
    pub final_state: CacheFinalState,
    pub finalize_ms: u64,
}

impl RunStore {
    pub async fn report_attempt_cache_preparations(
        &self,
        attempt_id: &str,
        token_hash: &str,
        authorization_ms: u64,
        wall_ms: u64,
        reports: Vec<AttemptCachePreparationCommand>,
        now_unix: u64,
    ) -> Result<Option<super::DispatchClaim>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let claim = authenticated_attempt(&tx, attempt_id, token_hash, now_unix).await?;
        let setup = AttemptCacheSetupObservation::new(attempt_id, authorization_ms, wall_ms)
            .map_err(PostgresError::from)?;
        let existing_setup =
            entities::run_attempt_cache_setup::Entity::find_by_id(attempt_id.to_string())
                .lock_exclusive()
                .one(&tx)
                .await
                .map_err(PostgresError::internal)?;
        let mut changed = if let Some(existing) = existing_setup {
            if existing.try_into_domain()? != setup {
                return Err(PostgresError::conflict(
                    "cache setup was already reported with different facts",
                ));
            }
            false
        } else {
            entities::run_attempt_cache_setup::Model::from_domain(&setup)?
                .into_active_model()
                .insert(&tx)
                .await
                .map_err(PostgresError::internal)?;
            true
        };
        let workflow_path = WorkflowPath::parse(claim.run.workflow.path().as_str().to_string())
            .map_err(PostgresError::invalid_input)?;
        let job_definition = claim
            .workflow_revision
            .definition()
            .job(&claim.job.key)
            .ok_or_else(|| {
                PostgresError::internal_message("run attempt job definition is missing")
            })?;

        // authenticated_attempt holds the attempt row lock until this transaction
        // commits, so concurrent exact retries serialize before checking this table.

        for report in reports {
            job_definition
                .caches()
                .iter()
                .find(|cache| cache.as_str() == report.cache_name)
                .ok_or_else(|| {
                    PostgresError::invalid_input(
                        "cache preparation does not belong to the claimed workflow job",
                    )
                })?;
            let observation = AttemptCacheObservation::prepared(
                attempt_id,
                workflow_path.clone(),
                claim.job.key.clone(),
                report.cache_name,
                report.identity_digest,
                report.preparation,
                AttemptCachePreparationTiming::new(
                    report.key_ms,
                    report.metadata_ms,
                    report.size_bytes,
                    report.download_verify_ms,
                    report.sync_ms,
                    report.extraction_ms,
                    report.prepare_ms,
                )
                .map_err(PostgresError::from)?,
            )
            .map_err(PostgresError::from)?;
            let existing = entities::run_attempt_cache::Entity::find_by_id((
                attempt_id.to_string(),
                observation.identity_digest.clone(),
            ))
            .lock_exclusive()
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?;
            if let Some(existing) = existing {
                if !existing
                    .try_into_domain()?
                    .has_same_preparation(&observation)
                {
                    return Err(PostgresError::conflict(
                        "cache preparation was already reported with different facts",
                    ));
                }
                continue;
            }
            entities::run_attempt_cache::Model::from_domain(&observation)?
                .into_active_model()
                .insert(&tx)
                .await
                .map_err(PostgresError::internal)?;
            changed = true;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(changed.then_some(claim))
    }

    pub async fn report_attempt_cache_finalizations(
        &self,
        attempt_id: &str,
        token_hash: &str,
        reports: Vec<AttemptCacheFinalizationCommand>,
        now_unix: u64,
    ) -> Result<Option<super::DispatchClaim>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let claim = authenticated_attempt(&tx, attempt_id, token_hash, now_unix).await?;
        let mut changed = false;
        for report in reports {
            let model = entities::run_attempt_cache::Entity::find_by_id((
                attempt_id.to_string(),
                report.identity_digest,
            ))
            .lock_exclusive()
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::conflict("cache preparation was not reported"))?;
            let mut observation = model.try_into_domain()?;
            if observation
                .finalize(report.final_state, report.finalize_ms)
                .map_err(PostgresError::from)?
            {
                entities::run_attempt_cache::Entity::update(
                    entities::run_attempt_cache::Model::from_domain(&observation)?
                        .into_active_model()
                        .reset_all(),
                )
                .exec(&tx)
                .await
                .map_err(PostgresError::internal)?;
                changed = true;
            }
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(changed.then_some(claim))
    }
}

async fn authenticated_attempt(
    tx: &sea_orm::DatabaseTransaction,
    attempt_id: &str,
    token_hash: &str,
    now_unix: u64,
) -> Result<super::DispatchClaim, PostgresError> {
    let (run, job, attempt, steps) =
        super::run_attempt_persistence::locked_attempt_context(tx, attempt_id).await?;
    attempt
        .authenticate_cache_observation_report(&job, token_hash, now_unix)
        .map_err(PostgresError::from)?;
    let workflow_revision = super::runs::workflow_revision_for_run(tx, &run).await?;
    Ok(super::DispatchClaim {
        run,
        job,
        attempt,
        steps,
        workflow_revision,
    })
}
