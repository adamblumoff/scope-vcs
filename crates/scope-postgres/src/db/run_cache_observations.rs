use super::{RunStore, entities};
use crate::error::PostgresError;
use scope_domain::runs::{
    cache::{
        AttemptCacheObservation, CacheFinalState, CacheIdentity, CacheNamespace, CachePlatform,
        CachePreparation,
    },
    workflow::WorkflowPath,
};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, QuerySelect, TransactionTrait};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptCachePreparationCommand {
    pub cache_name: String,
    pub identity_digest: String,
    pub preparation: CachePreparation,
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
        reports: Vec<AttemptCachePreparationCommand>,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let claim = authenticated_attempt(&tx, attempt_id, token_hash, now_unix).await?;
        let workflow_path = WorkflowPath::parse(claim.run.workflow.path().as_str().to_string())
            .map_err(PostgresError::invalid_input)?;
        let job_definition = claim
            .workflow_revision
            .definition()
            .job(&claim.job.key)
            .ok_or_else(|| {
                PostgresError::internal_message("run attempt job definition is missing")
            })?;
        let image = claim
            .job
            .pinned_container_image
            .as_ref()
            .ok_or_else(|| PostgresError::conflict("run job container image is not pinned"))?;
        let namespace = CacheNamespace::workflow(&workflow_path, &claim.job.key);

        for report in reports {
            let cache = job_definition
                .caches()
                .iter()
                .find(|cache| cache.as_str() == report.cache_name)
                .ok_or_else(|| {
                    PostgresError::invalid_input(
                        "cache preparation does not belong to the claimed workflow job",
                    )
                })?;
            let expected_digest = CacheIdentity::new(
                claim.run.workflow.repository_id(),
                namespace.clone(),
                cache.clone(),
                image,
                CachePlatform::LinuxAmd64,
            )
            .map_err(PostgresError::from)?
            .digest();
            if report.identity_digest != expected_digest {
                return Err(PostgresError::invalid_input(
                    "cache preparation identity does not match the claimed workflow job",
                ));
            }
            let observation = AttemptCacheObservation::prepared(
                attempt_id,
                workflow_path.clone(),
                claim.job.key.clone(),
                report.cache_name,
                report.identity_digest,
                report.preparation,
                report.prepare_ms,
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
        }
        tx.commit().await.map_err(PostgresError::internal)
    }

    pub async fn report_attempt_cache_finalizations(
        &self,
        attempt_id: &str,
        token_hash: &str,
        reports: Vec<AttemptCacheFinalizationCommand>,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        authenticated_attempt(&tx, attempt_id, token_hash, now_unix).await?;
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
            }
        }
        tx.commit().await.map_err(PostgresError::internal)
    }
}

async fn authenticated_attempt(
    tx: &sea_orm::DatabaseTransaction,
    attempt_id: &str,
    token_hash: &str,
    now_unix: u64,
) -> Result<super::DispatchClaim, PostgresError> {
    let (run_id, runner_id) =
        super::run_attempt_persistence::attempt_target(tx, attempt_id).await?;
    super::runner_protocol_cutover::guard_attempt_operation(tx, &runner_id, &run_id).await?;
    let (run, job, attempt, steps) =
        super::run_attempt_persistence::locked_attempt_context(tx, attempt_id).await?;
    super::run_attempt_persistence::ensure_runner_authorized(tx, &run, &attempt).await?;
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
        canary_phase: None,
    })
}
