use super::{
    RunStore, entities,
    object_references::insert_object_reference,
    run_attempt_persistence::{
        attempt_target, ensure_runner_authorized, jobs_for_run, locked_attempt_context,
        locked_attempt_steps, locked_heartbeat_context, locked_jobs, locked_run, runner_by_id,
        save_attempt, save_attempt_steps, save_job, save_jobs, save_run, save_runner,
    },
    runner_protocol_cutover::{
        guard_attempt_operation, guard_canary_pinned_image, guard_enqueue, guard_general_run_write,
        guard_runner_authentication, guard_runner_registration, record_canary_attempt_terminal,
    },
};
use crate::error::PostgresError;
use scope_domain::runs::{
    job::{RunJob, create_run_jobs, reconcile_run, request_run_cancellation, retry_run},
    run::{AttemptConclusion, PinnedContainerImage, Run, RunAttempt, RunAttemptStep, RunLogChunk},
    runner::{Runner, RunnerCapabilities, RunnerGrant, RunnerMaxConcurrentJobs},
    workflow::WorkflowRevision,
};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::{NotSet, Set},
    ColumnTrait, Condition, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, TransactionTrait, TryInsertResult,
    sea_query::OnConflict,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchClaim {
    pub run: Run,
    pub job: RunJob,
    pub attempt: RunAttempt,
    pub steps: Vec<RunAttemptStep>,
    pub workflow_revision: WorkflowRevision,
    pub canary_phase: Option<scope_domain::runs::cutover::RunnerProtocolCanaryPhase>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchOffer {
    pub run: Run,
    pub job: RunJob,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRunLog {
    pub position: u64,
    pub run_id: String,
    pub job_key: String,
    pub chunk: RunLogChunk,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpgradeRunnerRegistrationCommand {
    pub secret_hash: String,
    pub version: String,
    pub protocol_version: u32,
    pub capabilities: RunnerCapabilities,
    pub max_concurrent_jobs: RunnerMaxConcurrentJobs,
}

impl RunStore {
    pub async fn register_runner_with_grant(
        &self,
        runner: Runner,
        grant: RunnerGrant,
    ) -> Result<(Runner, RunnerGrant), PostgresError> {
        if runner.id != grant.runner_id || runner.owner_user_id != grant.granted_by_user_id {
            return Err(PostgresError::invalid_input(
                "runner registration and repository grant identities do not match",
            ));
        }
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        guard_runner_registration(&tx, &runner).await?;
        let runner_result = entities::runner::Entity::insert(
            entities::runner::Model::from_domain(&runner)?.into_active_model(),
        )
        .on_conflict(OnConflict::new().do_nothing().to_owned())
        .do_nothing()
        .exec(&tx)
        .await
        .map_err(PostgresError::internal)?;
        if !matches!(runner_result, TryInsertResult::Inserted(_)) {
            return Err(PostgresError::conflict(
                "runner id or secret hash is already registered",
            ));
        }
        let grant_result = entities::runner_grant::Entity::insert(
            entities::runner_grant::Model::from_domain(&grant)?.into_active_model(),
        )
        .on_conflict(OnConflict::new().do_nothing().to_owned())
        .do_nothing()
        .exec(&tx)
        .await
        .map_err(PostgresError::internal)?;
        if !matches!(grant_result, TryInsertResult::Inserted(_)) {
            return Err(PostgresError::conflict(
                "runner is already attached or the repository runner name is taken",
            ));
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok((runner, grant))
    }

    pub async fn register_runner(&self, runner: Runner) -> Result<Runner, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        guard_runner_registration(&tx, &runner).await?;
        let model = entities::runner::Model::from_domain(&runner)?;
        let result = entities::runner::Entity::insert(model.into_active_model())
            .on_conflict(OnConflict::new().do_nothing().to_owned())
            .do_nothing()
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        if !matches!(result, TryInsertResult::Inserted(_)) {
            return Err(PostgresError::conflict(
                "runner id or secret hash is already registered",
            ));
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(runner)
    }

    pub async fn upgrade_runner_registration(
        &self,
        runner_id: &str,
        owner_user_id: &str,
        command: UpgradeRunnerRegistrationCommand,
    ) -> Result<Runner, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let current = runner_by_id(&tx, runner_id).await?;
        if current.owner_user_id != owner_user_id {
            return Err(PostgresError::not_found("runner not found"));
        }
        let upgraded = Runner::restore(
            current.id,
            current.owner_user_id,
            command.secret_hash,
            command.version,
            command.protocol_version,
            command.capabilities,
            command.max_concurrent_jobs,
            true,
            current.created_at_unix,
            current.last_seen_at_unix,
        )
        .map_err(PostgresError::from)?;
        guard_runner_registration(&tx, &upgraded).await?;
        save_runner(&tx, &upgraded).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(upgraded)
    }

    pub async fn set_runner_enabled(
        &self,
        runner_id: &str,
        enabled: bool,
    ) -> Result<Runner, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let mut runner = runner_by_id(&tx, runner_id).await?;
        runner.set_enabled(enabled);
        entities::runner::Entity::update(
            entities::runner::Model::from_domain(&runner)?
                .into_active_model()
                .reset_all(),
        )
        .exec(&tx)
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(runner)
    }

    pub async fn runner(&self, runner_id: &str) -> Result<Option<Runner>, PostgresError> {
        entities::runner::Entity::find_by_id(runner_id.to_string())
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .map(entities::runner::Model::try_into_domain)
            .transpose()
    }

    pub async fn delete_unused_runner(&self, runner_id: &str) -> Result<(), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        runner_by_id(&tx, runner_id).await?;
        if entities::run_attempt::Entity::find()
            .filter(entities::run_attempt::Column::RunnerId.eq(runner_id))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .is_some()
        {
            return Err(PostgresError::conflict(
                "runner with attempts cannot be deleted",
            ));
        }
        entities::runner_grant::Entity::delete_many()
            .filter(entities::runner_grant::Column::RunnerId.eq(runner_id))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::runner::Entity::delete_by_id(runner_id.to_string())
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn authenticate_runner(
        &self,
        secret_hash: &str,
        now_unix: u64,
    ) -> Result<Runner, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let model = entities::runner::Entity::find()
            .filter(entities::runner::Column::SecretHash.eq(secret_hash))
            .lock_exclusive()
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::unauthenticated("runner credentials are invalid"))?;
        let mut runner = model.try_into_domain()?;
        guard_runner_authentication(&tx, &runner).await?;
        runner.record_seen(now_unix).map_err(PostgresError::from)?;
        save_runner(&tx, &runner).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(runner)
    }

    pub async fn runner_grants(&self, runner_id: &str) -> Result<Vec<RunnerGrant>, PostgresError> {
        entities::runner_grant::Entity::find()
            .filter(entities::runner_grant::Column::RunnerId.eq(runner_id))
            .order_by_asc(entities::runner_grant::Column::RepoId)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::runner_grant::Model::try_into_domain)
            .collect()
    }

    pub async fn grant_runner(&self, grant: RunnerGrant) -> Result<RunnerGrant, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        runner_by_id(&tx, &grant.runner_id).await?;
        if let Some(existing) = entities::runner_grant::Entity::find_by_id((
            grant.repository_id.clone(),
            grant.runner_id.clone(),
        ))
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(PostgresError::internal)?
        {
            let existing = existing.try_into_domain()?;
            if existing.is_active() {
                return Err(PostgresError::conflict(
                    "runner is already attached to the repository",
                ));
            }
            if has_active_attempts_for_grant(&tx, &grant.repository_id, &grant.runner_id).await? {
                return Err(PostgresError::conflict(
                    "runner cannot be reattached until attempts from the revoked grant are terminal",
                ));
            }
            entities::runner_grant::Entity::update(
                entities::runner_grant::Model::from_domain(&grant)?
                    .into_active_model()
                    .reset_all(),
            )
            .exec(&tx)
            .await
            .map_err(|error| unique_conflict(error, "repository runner name is already in use"))?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(grant);
        }

        let model = entities::runner_grant::Model::from_domain(&grant)?;
        let result = entities::runner_grant::Entity::insert(model.into_active_model())
            .on_conflict(OnConflict::new().do_nothing().to_owned())
            .do_nothing()
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        if !matches!(result, TryInsertResult::Inserted(_)) {
            return Err(PostgresError::conflict(
                "runner is already attached or the repository runner name is taken",
            ));
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(grant)
    }

    pub async fn revoke_runner_grant(
        &self,
        repository_id: &str,
        runner_id: &str,
        now_unix: u64,
    ) -> Result<RunnerGrant, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let grant = entities::runner_grant::Entity::find_by_id((
            repository_id.to_string(),
            runner_id.to_string(),
        ))
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("runner grant not found"))?;
        let mut grant = grant.try_into_domain()?;
        if grant.revoke(now_unix).map_err(PostgresError::from)? {
            entities::runner_grant::Entity::update(
                entities::runner_grant::Model::from_domain(&grant)?
                    .into_active_model()
                    .reset_all(),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(grant)
    }

    pub async fn enqueue_run(
        &self,
        run: Run,
        revision: WorkflowRevision,
    ) -> Result<Run, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let stored = enqueue_run_in_transaction(&tx, run, revision).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(stored)
    }

    pub async fn pin_attempt_container_image(
        &self,
        attempt_id: &str,
        runner_id: &str,
        token_hash: &str,
        image: PinnedContainerImage,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (guard_run_id, guard_runner_id) = attempt_target(&tx, attempt_id).await?;
        guard_attempt_operation(&tx, &guard_runner_id, &guard_run_id).await?;
        let (run, mut job, attempt, steps) = locked_attempt_context(&tx, attempt_id).await?;
        ensure_runner_authorized(&tx, &run, &attempt).await?;
        attempt
            .authenticate_access(&job, token_hash, now_unix)
            .map_err(PostgresError::from)?;
        if attempt.runner_id != runner_id {
            return Err(PostgresError::permission_denied(
                "attempt runner identity does not match",
            ));
        }
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        guard_canary_pinned_image(&tx, runner_id, &run.id, &workflow_revision, &image).await?;
        job.pin_container_image(image, now_unix)
            .map_err(PostgresError::from)?;
        save_job(&tx, &job).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            job,
            attempt,
            steps,
            workflow_revision,
            canary_phase: None,
        })
    }

    pub async fn heartbeat_attempt(
        &self,
        attempt_id: &str,
        runner_id: &str,
        token_hash: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<bool, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (guard_run_id, guard_runner_id) = attempt_target(&tx, attempt_id).await?;
        guard_attempt_operation(&tx, &guard_runner_id, &guard_run_id).await?;
        let (run, job, mut attempt) = locked_heartbeat_context(&tx, attempt_id).await?;
        let mut runner = ensure_runner_authorized(&tx, &run, &attempt).await?;
        runner.record_seen(now_unix).map_err(PostgresError::from)?;
        let observed_now = runner
            .last_seen_at_unix
            .ok_or_else(|| PostgresError::internal_message("runner observation time is missing"))?;
        let cancellation_requested = attempt
            .heartbeat(
                &run,
                &job,
                runner_id,
                token_hash,
                observed_now,
                lease_expires_at_unix,
            )
            .map_err(PostgresError::from)?;
        save_attempt(&tx, &attempt).await?;
        save_runner(&tx, &runner).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(cancellation_requested)
    }

    pub async fn complete_attempt(
        &self,
        attempt_id: &str,
        token_hash: &str,
        conclusion: AttemptConclusion,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        self.mutate_attempt(attempt_id, |run, job, attempt, steps| {
            let runner_id = attempt.runner_id.clone();
            attempt.complete(
                run, job, steps, &runner_id, token_hash, conclusion, now_unix,
            )
        })
        .await
    }

    pub async fn abandon_attempt(
        &self,
        attempt_id: &str,
        runner_id: &str,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        self.mutate_attempt(attempt_id, |run, job, attempt, steps| {
            attempt.abandon(run, job, steps, runner_id, token_hash, now_unix)
        })
        .await
    }

    pub async fn expire_attempt(
        &self,
        attempt_id: &str,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (guard_run_id, guard_runner_id) = attempt_target(&tx, attempt_id).await?;
        guard_attempt_operation(&tx, &guard_runner_id, &guard_run_id).await?;
        let mut jobs = locked_jobs(&tx, &guard_run_id).await?;
        let mut run = locked_run(&tx, &guard_run_id).await?;
        let mut attempt = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
            .lock_exclusive()
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("run attempt not found"))?
            .try_into_domain()?;
        let mut steps = locked_attempt_steps(&tx, attempt_id).await?;
        let job = jobs
            .iter_mut()
            .find(|job| job.key == attempt.job_key)
            .ok_or_else(|| PostgresError::internal_message("run attempt job is missing"))?;
        attempt
            .expire(&run, job, &mut steps, now_unix)
            .map_err(PostgresError::from)?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        reconcile_run(&mut run, &mut jobs, &workflow_revision, now_unix)
            .map_err(PostgresError::from)?;
        save_attempt(&tx, &attempt).await?;
        save_attempt_steps(&tx, &steps).await?;
        save_jobs(&tx, &jobs).await?;
        save_run(&tx, &run).await?;
        record_canary_attempt_terminal(&tx, &attempt.runner_id, &run.id, attempt.state, now_unix)
            .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        let job = jobs
            .into_iter()
            .find(|job| job.key == attempt.job_key)
            .ok_or_else(|| PostgresError::internal_message("run attempt job is missing"))?;
        Ok(DispatchClaim {
            run,
            job,
            attempt,
            steps,
            workflow_revision,
            canary_phase: None,
        })
    }

    pub async fn expired_attempt_ids(
        &self,
        now_unix: u64,
        limit: u64,
    ) -> Result<Vec<String>, PostgresError> {
        let now_unix = entities::u64_to_i64(now_unix, "attempt recovery time")?;
        Ok(entities::run_attempt::Entity::find()
            .filter(
                entities::run_attempt::Column::State
                    .is_in(["leased".to_string(), "running".to_string()]),
            )
            .filter(entities::run_attempt::Column::LeaseExpiresAtUnix.lte(now_unix))
            .order_by_asc(entities::run_attempt::Column::LeaseExpiresAtUnix)
            .order_by_asc(entities::run_attempt::Column::Id)
            .limit(limit)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|attempt| attempt.id)
            .collect())
    }

    pub async fn request_run_cancellation(
        &self,
        run_id: &str,
        now_unix: u64,
    ) -> Result<Run, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        guard_general_run_write(&tx).await?;
        let mut jobs = locked_jobs(&tx, run_id).await?;
        let mut run = locked_run(&tx, run_id).await?;
        request_run_cancellation(&mut run, &mut jobs, now_unix).map_err(PostgresError::from)?;
        save_jobs(&tx, &jobs).await?;
        save_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(run)
    }

    pub async fn retry_run(&self, run_id: &str, now_unix: u64) -> Result<Run, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        guard_general_run_write(&tx).await?;
        let mut jobs = locked_jobs(&tx, run_id).await?;
        let mut run = locked_run(&tx, run_id).await?;
        let revision = workflow_revision_for_run(&tx, &run).await?;
        retry_run(&mut run, &mut jobs, &revision, now_unix).map_err(PostgresError::from)?;
        save_jobs(&tx, &jobs).await?;
        save_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(run)
    }

    pub async fn run(&self, run_id: &str) -> Result<Option<Run>, PostgresError> {
        entities::run::Entity::find_by_id(run_id.to_string())
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .map(entities::run::Model::try_into_domain)
            .transpose()
    }

    pub async fn authenticate_attempt(
        &self,
        attempt_id: &str,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (guard_run_id, guard_runner_id) = attempt_target(&tx, attempt_id).await?;
        guard_attempt_operation(&tx, &guard_runner_id, &guard_run_id).await?;
        let (run, job, attempt, steps) = locked_attempt_context(&tx, attempt_id).await?;
        ensure_runner_authorized(&tx, &run, &attempt).await?;
        attempt
            .authenticate_access(&job, token_hash, now_unix)
            .map_err(PostgresError::from)?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            job,
            attempt,
            steps,
            workflow_revision,
            canary_phase: None,
        })
    }

    pub async fn append_attempt_log(
        &self,
        chunk: RunLogChunk,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<StoredRunLog, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (guard_run_id, guard_runner_id) = attempt_target(&tx, &chunk.attempt_id).await?;
        guard_attempt_operation(&tx, &guard_runner_id, &guard_run_id).await?;
        let (run, job, mut attempt, steps) = locked_attempt_context(&tx, &chunk.attempt_id).await?;
        ensure_runner_authorized(&tx, &run, &attempt).await?;
        attempt
            .authenticate_access(&job, token_hash, now_unix)
            .map_err(PostgresError::from)?;
        if attempt.logs_truncated {
            return Err(PostgresError::resource_exhausted(
                "run attempt log limit reached",
            ));
        }

        if let Some(existing) = entities::run_log::Entity::find()
            .filter(entities::run_log::Column::AttemptId.eq(&chunk.attempt_id))
            .filter(
                entities::run_log::Column::Sequence.eq(i64::try_from(chunk.sequence)
                    .map_err(|_| PostgresError::invalid_input("run log sequence is too large"))?),
            )
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        {
            let position = entities::i64_to_u64(existing.position, "run log position")?;
            let existing_run_id = existing.run_id.clone();
            let existing_chunk = existing.try_into_domain()?;
            if existing_run_id != run.id
                || existing_chunk.attempt_id != chunk.attempt_id
                || existing_chunk.step_index != chunk.step_index
                || existing_chunk.sequence != chunk.sequence
                || existing_chunk.text != chunk.text
            {
                return Err(PostgresError::conflict(
                    "run log sequence is already used by different content",
                ));
            }
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(StoredRunLog {
                position,
                run_id: existing_run_id,
                job_key: job.key.as_str().to_string(),
                chunk: existing_chunk,
            });
        }

        let expected_sequence = entities::run_log::Entity::find()
            .filter(entities::run_log::Column::AttemptId.eq(&chunk.attempt_id))
            .order_by_desc(entities::run_log::Column::Sequence)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .map_or(1, |last| last.sequence.saturating_add(1));
        if i64::try_from(chunk.sequence).ok() != Some(expected_sequence) {
            return Err(PostgresError::conflict(
                "run log sequence must append without gaps",
            ));
        }
        if !attempt
            .accept_log_chunk(&steps, &chunk)
            .map_err(PostgresError::from)?
        {
            save_attempt(&tx, &attempt).await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Err(PostgresError::resource_exhausted(
                "run attempt log limit reached",
            ));
        }

        let mut model = entities::run_log::Model::from_domain(&run.id, &chunk)?.into_active_model();
        model.position = NotSet;
        let inserted = entities::run_log::Entity::insert(model)
            .exec(&tx)
            .await
            .map_err(|error| unique_conflict(error, "run log sequence is already in use"))?;
        let position = entities::i64_to_u64(inserted.last_insert_id, "run log position")?;
        save_attempt(&tx, &attempt).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(StoredRunLog {
            position,
            run_id: run.id,
            job_key: job.key.as_str().to_string(),
            chunk,
        })
    }

    pub(super) async fn mutate_attempt(
        &self,
        attempt_id: &str,
        mutate: impl FnOnce(
            &Run,
            &mut RunJob,
            &mut RunAttempt,
            &mut [RunAttemptStep],
        ) -> Result<(), scope_domain::error::DomainError>,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (guard_run_id, guard_runner_id) = attempt_target(&tx, attempt_id).await?;
        guard_attempt_operation(&tx, &guard_runner_id, &guard_run_id).await?;
        let mut jobs = locked_jobs(&tx, &guard_run_id).await?;
        let mut run = locked_run(&tx, &guard_run_id).await?;
        let mut attempt = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
            .lock_exclusive()
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("run attempt not found"))?
            .try_into_domain()?;
        let mut steps = locked_attempt_steps(&tx, attempt_id).await?;
        ensure_runner_authorized(&tx, &run, &attempt).await?;
        let job = jobs
            .iter_mut()
            .find(|job| job.key == attempt.job_key)
            .ok_or_else(|| PostgresError::internal_message("run attempt job is missing"))?;
        mutate(&run, job, &mut attempt, &mut steps).map_err(PostgresError::from)?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        let transition_time = attempt.completed_at_unix.unwrap_or(job.updated_at_unix);
        reconcile_run(&mut run, &mut jobs, &workflow_revision, transition_time)
            .map_err(PostgresError::from)?;
        save_attempt(&tx, &attempt).await?;
        save_attempt_steps(&tx, &steps).await?;
        save_jobs(&tx, &jobs).await?;
        save_run(&tx, &run).await?;
        record_canary_attempt_terminal(
            &tx,
            &attempt.runner_id,
            &run.id,
            attempt.state,
            attempt.completed_at_unix.unwrap_or(run.updated_at_unix),
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        let job = jobs
            .into_iter()
            .find(|job| job.key == attempt.job_key)
            .ok_or_else(|| PostgresError::internal_message("run attempt job is missing"))?;
        Ok(DispatchClaim {
            run,
            job,
            attempt,
            steps,
            workflow_revision,
            canary_phase: None,
        })
    }

    pub(super) async fn mutate_active_attempt(
        &self,
        attempt_id: &str,
        mutate: impl FnOnce(
            &Run,
            &mut RunJob,
            &mut RunAttempt,
            &mut [RunAttemptStep],
        ) -> Result<(), scope_domain::error::DomainError>,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (guard_run_id, guard_runner_id) = attempt_target(&tx, attempt_id).await?;
        guard_attempt_operation(&tx, &guard_runner_id, &guard_run_id).await?;
        let (run_snapshot, mut job, mut attempt, mut steps) =
            locked_attempt_context(&tx, attempt_id).await?;
        ensure_runner_authorized(&tx, &run_snapshot, &attempt).await?;
        let mut run = locked_run(&tx, &run_snapshot.id).await?;
        mutate(&run, &mut job, &mut attempt, &mut steps).map_err(PostgresError::from)?;
        save_job(&tx, &job).await?;
        save_attempt(&tx, &attempt).await?;
        save_attempt_steps(&tx, &steps).await?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        let mut jobs = jobs_for_run(&tx, &run.id).await?;
        if let Some(stored) = jobs.iter_mut().find(|stored| stored.key == job.key) {
            *stored = job.clone();
        }
        reconcile_run(&mut run, &mut jobs, &workflow_revision, job.updated_at_unix)
            .map_err(PostgresError::from)?;
        save_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            job,
            attempt,
            steps,
            workflow_revision,
            canary_phase: None,
        })
    }
}

pub(super) async fn enqueue_run_in_transaction(
    tx: &DatabaseTransaction,
    run: Run,
    revision: WorkflowRevision,
) -> Result<Run, PostgresError> {
    guard_enqueue(tx, &run, &revision).await?;
    run.validate_workflow_revision(&revision)
        .map_err(PostgresError::from)?;
    let requested_jobs = create_run_jobs(&run, &revision).map_err(PostgresError::from)?;
    save_workflow_revision(tx, &revision, run.created_at_unix).await?;
    let model = entities::run::Model::from_domain(&run)?;
    let result = entities::run::Entity::insert(model.into_active_model())
        .on_conflict(OnConflict::new().do_nothing().to_owned())
        .do_nothing()
        .exec(tx)
        .await
        .map_err(PostgresError::internal)?;
    let inserted = matches!(result, TryInsertResult::Inserted(_));
    if !inserted {
        let stored = entities::run::Entity::find()
            .filter(entities::run::Column::RepoId.eq(run.workflow.repository_id().to_string()))
            .filter(entities::run::Column::IdempotencyKey.eq(run.idempotency_key.clone()))
            .one(tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| {
                PostgresError::conflict("run id is already used by another idempotency key")
            })?
            .try_into_domain()?;
        let stored_jobs = jobs_for_run(tx, &stored.id).await?;
        if !stored.has_same_enqueue_request_identity(&run)
            || !has_same_effective_runner_request(&stored_jobs, &requested_jobs)
        {
            return Err(PostgresError::conflict(
                "run idempotency key is already used by a different enqueue request",
            ));
        }
        return Ok(stored);
    }
    entities::run_job::Entity::insert_many(
        requested_jobs
            .iter()
            .map(entities::run_job::Model::from_domain)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(IntoActiveModel::into_active_model),
    )
    .exec(tx)
    .await
    .map_err(PostgresError::internal)?;
    for object in run.source.retained_objects() {
        insert_object_reference(tx, "run_source", &run.id, object).await?;
    }
    Ok(run)
}

fn has_same_effective_runner_request(stored: &[RunJob], requested: &[RunJob]) -> bool {
    stored.len() == requested.len()
        && stored.iter().all(|stored_job| {
            requested.iter().any(|requested_job| {
                stored_job.key == requested_job.key
                    && stored_job.desired_runner == requested_job.desired_runner
            })
        })
}

async fn save_workflow_revision(
    tx: &DatabaseTransaction,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), PostgresError> {
    entities::workflow_revision::Entity::insert(
        entities::workflow_revision::Model::from_domain(revision, created_at_unix)?
            .into_active_model(),
    )
    .on_conflict(
        OnConflict::column(entities::workflow_revision::Column::Digest)
            .do_nothing()
            .to_owned(),
    )
    .do_nothing()
    .exec(tx)
    .await
    .map_err(PostgresError::internal)?;
    let persisted = entities::workflow_revision::Entity::find_by_id(revision.digest().to_string())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("workflow revision was not stored"))?;
    let persisted = persisted.try_into_domain(revision.workflow().clone())?;
    if &persisted != revision {
        return Err(PostgresError::conflict(
            "workflow revision digest is already used by a different definition",
        ));
    }
    Ok(())
}

pub(super) async fn workflow_revision_for_run(
    tx: &DatabaseTransaction,
    run: &Run,
) -> Result<WorkflowRevision, PostgresError> {
    entities::workflow_revision::Entity::find_by_id(run.workflow_revision_digest.clone())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("run workflow revision is missing"))?
        .try_into_domain(run.workflow.clone())
}

pub(super) async fn revoke_runner_grants_owned_by<C>(
    conn: &C,
    repository_id: &str,
    owner_user_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let runner_ids = entities::runner::Entity::find()
        .select_only()
        .column(entities::runner::Column::Id)
        .filter(entities::runner::Column::OwnerUserId.eq(owner_user_id))
        .into_tuple::<String>()
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    if runner_ids.is_empty() {
        return Ok(());
    }
    entities::runner_grant::Entity::update_many()
        .set(entities::runner_grant::ActiveModel {
            revoked_at_unix: Set(Some(entities::u64_to_i64(
                now_unix,
                "runner grant revocation time",
            )?)),
            ..Default::default()
        })
        .filter(entities::runner_grant::Column::RepoId.eq(repository_id))
        .filter(entities::runner_grant::Column::RunnerId.is_in(runner_ids))
        .filter(entities::runner_grant::Column::RevokedAtUnix.is_null())
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    Ok(())
}

async fn has_active_attempts_for_grant(
    tx: &DatabaseTransaction,
    repository_id: &str,
    runner_id: &str,
) -> Result<bool, PostgresError> {
    let run_ids = entities::run::Entity::find()
        .select_only()
        .column(entities::run::Column::Id)
        .filter(entities::run::Column::RepoId.eq(repository_id))
        .into_tuple::<String>()
        .all(tx)
        .await
        .map_err(PostgresError::internal)?;
    if run_ids.is_empty() {
        return Ok(false);
    }
    entities::run_attempt::Entity::find()
        .filter(entities::run_attempt::Column::RunId.is_in(run_ids))
        .filter(entities::run_attempt::Column::RunnerId.eq(runner_id))
        .filter(
            Condition::any()
                .add(entities::run_attempt::Column::State.eq("leased"))
                .add(entities::run_attempt::Column::State.eq("running")),
        )
        .one(tx)
        .await
        .map(|attempt| attempt.is_some())
        .map_err(PostgresError::internal)
}

pub(super) fn unique_conflict(error: DbErr, message: &str) -> PostgresError {
    if matches!(
        error.sql_err(),
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
    ) {
        PostgresError::conflict(message)
    } else {
        PostgresError::internal(error)
    }
}
