use super::{
    GeneratedIdSource, RunStore, cleanup_queue::queue_pending_source_blob_deletion_rows, entities,
    object_references::insert_object_reference,
};
use crate::error::PostgresError;
use scope_domain::runs::{
    run::{AttemptConclusion, PinnedContainerImage, Run, RunAttempt, RunLogChunk},
    runner::{Runner, RunnerGrant},
    workflow::WorkflowRevision,
};
use scope_domain::store::SourceBlob;
use sea_orm::{
    ActiveModelTrait, ActiveValue::NotSet, ColumnTrait, Condition, DatabaseTransaction, DbErr,
    EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
    TryInsertResult, sea_query::OnConflict,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchClaim {
    pub run: Run,
    pub attempt: RunAttempt,
    pub workflow_revision: WorkflowRevision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRunLog {
    pub position: u64,
    pub run_id: String,
    pub chunk: RunLogChunk,
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
        let model = entities::runner::Model::from_domain(&runner)?;
        let result = entities::runner::Entity::insert(model.into_active_model())
            .on_conflict(OnConflict::new().do_nothing().to_owned())
            .do_nothing()
            .exec(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        if !matches!(result, TryInsertResult::Inserted(_)) {
            return Err(PostgresError::conflict(
                "runner id or secret hash is already registered",
            ));
        }
        Ok(runner)
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
        if !runner.supports_dispatch() {
            return Err(PostgresError::permission_denied(
                "runner is disabled or incompatible with the V1 protocol",
            ));
        }
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
        run.validate_workflow_revision(&revision)
            .map_err(PostgresError::from)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        save_workflow_revision(&tx, &revision, run.created_at_unix).await?;
        let model = entities::run::Model::from_domain(&run)?;
        let result = entities::run::Entity::insert(model.into_active_model())
            .on_conflict(OnConflict::new().do_nothing().to_owned())
            .do_nothing()
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let inserted = matches!(result, TryInsertResult::Inserted(_));
        let stored = if !inserted {
            let stored = entities::run::Entity::find()
                .filter(entities::run::Column::RepoId.eq(run.workflow.repository_id().to_string()))
                .filter(entities::run::Column::IdempotencyKey.eq(run.idempotency_key.clone()))
                .one(&tx)
                .await
                .map_err(PostgresError::internal)?
                .ok_or_else(|| {
                    PostgresError::conflict("run id is already used by another idempotency key")
                })?
                .try_into_domain()?;
            if !stored.has_same_enqueue_request(&run) {
                return Err(PostgresError::conflict(
                    "run idempotency key is already used by a different enqueue request",
                ));
            }
            stored
        } else {
            for object in run.source.retained_objects() {
                insert_object_reference(&tx, "run_source", &run.id, object).await?;
            }
            run
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(stored)
    }

    pub async fn next_dispatchable_run(
        &self,
        runner_id: &str,
    ) -> Result<Option<Run>, PostgresError> {
        let runner = entities::runner::Entity::find_by_id(runner_id.to_string())
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("runner not found"))?
            .try_into_domain()?;
        if !runner.supports_dispatch() {
            return Ok(None);
        }

        let grants = entities::runner_grant::Entity::find()
            .filter(entities::runner_grant::Column::RunnerId.eq(runner_id))
            .filter(entities::runner_grant::Column::RevokedAtUnix.is_null())
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        if grants.is_empty() {
            return Ok(None);
        }
        let eligible = grants
            .into_iter()
            .fold(Condition::any(), |eligible, grant| {
                eligible.add(
                    Condition::all()
                        .add(entities::run::Column::RepoId.eq(grant.repo_id))
                        .add(
                            Condition::any()
                                .add(entities::run::Column::DesiredRunnerName.is_null())
                                .add(entities::run::Column::DesiredRunnerName.eq(grant.name)),
                        ),
                )
            });

        entities::run::Entity::find()
            .filter(entities::run::Column::State.eq("queued"))
            .filter(entities::run::Column::CancellationRequested.eq(false))
            .filter(eligible)
            .order_by_asc(entities::run::Column::CreatedAtUnix)
            .order_by_asc(entities::run::Column::Id)
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .map(entities::run::Model::try_into_domain)
            .transpose()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn claim_run(
        &self,
        run_id: &str,
        runner_id: &str,
        attempt_id: &str,
        token_hash: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let mut run = locked_run(&tx, run_id).await?;
        let mut runner = runner_by_id(&tx, runner_id).await?;
        let grant = grant_by_ids(&tx, run.workflow.repository_id(), runner_id).await?;
        let attempt = run
            .claim(
                &runner,
                &grant,
                attempt_id,
                token_hash,
                now_unix,
                lease_expires_at_unix,
            )
            .map_err(PostgresError::from)?;
        runner.record_seen(now_unix).map_err(PostgresError::from)?;

        entities::run_attempt::Entity::insert(
            entities::run_attempt::Model::from_domain(&attempt)?.into_active_model(),
        )
        .exec(&tx)
        .await
        .map_err(|error| {
            unique_conflict(error, "run attempt id or token hash is already in use")
        })?;
        save_run(&tx, &run).await?;
        save_runner(&tx, &runner).await?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            attempt,
            workflow_revision,
        })
    }

    pub async fn start_attempt(
        &self,
        attempt_id: &str,
        runner_id: &str,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        self.mutate_attempt(attempt_id, |run, attempt| {
            attempt.start(run, runner_id, token_hash, now_unix)
        })
        .await
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
        let (mut run, attempt) = locked_attempt_context(&tx, attempt_id).await?;
        ensure_runner_authorized(&tx, &run, &attempt).await?;
        attempt
            .authenticate_access(&run, token_hash, now_unix)
            .map_err(PostgresError::from)?;
        if attempt.runner_id != runner_id {
            return Err(PostgresError::permission_denied(
                "attempt runner identity does not match",
            ));
        }
        run.pin_container_image(image, now_unix)
            .map_err(PostgresError::from)?;
        save_run(&tx, &run).await?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            attempt,
            workflow_revision,
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
        let (run, mut attempt) = locked_attempt_context(&tx, attempt_id).await?;
        let mut runner = ensure_runner_authorized(&tx, &run, &attempt).await?;
        let cancellation_requested = attempt
            .heartbeat(&run, runner_id, token_hash, now_unix, lease_expires_at_unix)
            .map_err(PostgresError::from)?;
        save_attempt(&tx, &attempt).await?;
        runner.record_seen(now_unix).map_err(PostgresError::from)?;
        save_runner(&tx, &runner).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(cancellation_requested)
    }

    pub async fn complete_attempt(
        &self,
        attempt_id: &str,
        runner_id: &str,
        token_hash: &str,
        conclusion: AttemptConclusion,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        self.mutate_attempt(attempt_id, |run, attempt| {
            attempt.complete(run, runner_id, token_hash, conclusion, now_unix)
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
        self.mutate_attempt(attempt_id, |run, attempt| {
            attempt.abandon(run, runner_id, token_hash, now_unix)
        })
        .await
    }

    pub async fn expire_attempt(
        &self,
        attempt_id: &str,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (mut run, mut attempt) = locked_attempt_context(&tx, attempt_id).await?;
        attempt
            .expire(&mut run, now_unix)
            .map_err(PostgresError::from)?;
        save_attempt(&tx, &attempt).await?;
        save_run(&tx, &run).await?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            attempt,
            workflow_revision,
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
        let mut run = locked_run(&tx, run_id).await?;
        run.request_cancellation(now_unix)
            .map_err(PostgresError::from)?;
        save_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(run)
    }

    pub async fn retry_run(&self, run_id: &str, now_unix: u64) -> Result<Run, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let mut run = locked_run(&tx, run_id).await?;
        run.retry(now_unix).map_err(PostgresError::from)?;
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
        let (run, attempt) = locked_attempt_context(&tx, attempt_id).await?;
        ensure_runner_authorized(&tx, &run, &attempt).await?;
        attempt
            .authenticate_access(&run, token_hash, now_unix)
            .map_err(PostgresError::from)?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            attempt,
            workflow_revision,
        })
    }

    pub async fn append_attempt_log(
        &self,
        chunk: RunLogChunk,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<StoredRunLog, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (run, mut attempt) = locked_attempt_context(&tx, &chunk.attempt_id).await?;
        ensure_runner_authorized(&tx, &run, &attempt).await?;
        attempt
            .authenticate_access(&run, token_hash, now_unix)
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
            .accept_log_chunk(&chunk)
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
            chunk,
        })
    }

    pub async fn run_logs_after(
        &self,
        run_id: &str,
        after: u64,
        limit: u64,
    ) -> Result<Vec<StoredRunLog>, PostgresError> {
        let after = i64::try_from(after)
            .map_err(|_| PostgresError::invalid_input("run log cursor is too large"))?;
        entities::run_log::Entity::find()
            .filter(entities::run_log::Column::RunId.eq(run_id))
            .filter(entities::run_log::Column::Position.gt(after))
            .order_by_asc(entities::run_log::Column::Position)
            .limit(limit)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|model| {
                let position = entities::i64_to_u64(model.position, "run log position")?;
                let run_id = model.run_id.clone();
                Ok(StoredRunLog {
                    position,
                    run_id,
                    chunk: model.try_into_domain()?,
                })
            })
            .collect()
    }

    pub async fn prune_terminal_runs(
        &self,
        completed_before_unix: u64,
        now_unix: u64,
        limit: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<usize, PostgresError> {
        let cutoff = entities::u64_to_i64(completed_before_unix, "run retention cutoff")?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let models = entities::run::Entity::find()
            .filter(entities::run::Column::State.is_in([
                "succeeded".to_string(),
                "failed".to_string(),
                "canceled".to_string(),
                "lost".to_string(),
            ]))
            .filter(entities::run::Column::CompletedAtUnix.lte(cutoff))
            .order_by_asc(entities::run::Column::CompletedAtUnix)
            .order_by_asc(entities::run::Column::Id)
            .limit(limit)
            .lock_exclusive()
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let runs = models
            .into_iter()
            .map(entities::run::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        let run_ids = runs.iter().map(|run| run.id.clone()).collect::<Vec<_>>();
        if run_ids.is_empty() {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(0);
        }
        let sources = runs
            .iter()
            .flat_map(|run| run.source.retained_objects())
            .cloned()
            .collect::<Vec<SourceBlob>>();

        entities::run_log::Entity::delete_many()
            .filter(entities::run_log::Column::RunId.is_in(run_ids.clone()))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::run_attempt::Entity::delete_many()
            .filter(entities::run_attempt::Column::RunId.is_in(run_ids.clone()))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::object_reference::Entity::delete_many()
            .filter(entities::object_reference::Column::RefKind.eq("run_source"))
            .filter(entities::object_reference::Column::RefId.is_in(run_ids.clone()))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::run::Entity::delete_many()
            .filter(entities::run::Column::Id.is_in(run_ids))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        queue_pending_source_blob_deletion_rows(&tx, sources.clone(), now_unix, generated_ids)
            .await?;
        tx.commit().await.map_err(PostgresError::internal)?;

        Ok(sources.len())
    }

    async fn mutate_attempt(
        &self,
        attempt_id: &str,
        mutate: impl FnOnce(&mut Run, &mut RunAttempt) -> Result<(), scope_domain::error::DomainError>,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (mut run, mut attempt) = locked_attempt_context(&tx, attempt_id).await?;
        ensure_runner_authorized(&tx, &run, &attempt).await?;
        mutate(&mut run, &mut attempt).map_err(PostgresError::from)?;
        save_attempt(&tx, &attempt).await?;
        save_run(&tx, &run).await?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchClaim {
            run,
            attempt,
            workflow_revision,
        })
    }
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

async fn workflow_revision_for_run(
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

async fn locked_run(tx: &DatabaseTransaction, run_id: &str) -> Result<Run, PostgresError> {
    entities::run::Entity::find_by_id(run_id.to_string())
        .lock_exclusive()
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("run not found"))?
        .try_into_domain()
}

async fn locked_attempt_context(
    tx: &DatabaseTransaction,
    attempt_id: &str,
) -> Result<(Run, RunAttempt), PostgresError> {
    let run_id = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("run attempt not found"))?
        .run_id;
    let run = locked_run(tx, &run_id).await?;
    let attempt = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
        .lock_exclusive()
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("run attempt not found"))?
        .try_into_domain()?;
    Ok((run, attempt))
}

async fn runner_by_id(tx: &DatabaseTransaction, runner_id: &str) -> Result<Runner, PostgresError> {
    entities::runner::Entity::find_by_id(runner_id.to_string())
        .lock_exclusive()
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("runner not found"))?
        .try_into_domain()
}

async fn grant_by_ids(
    tx: &DatabaseTransaction,
    repository_id: &str,
    runner_id: &str,
) -> Result<RunnerGrant, PostgresError> {
    entities::runner_grant::Entity::find_by_id((repository_id.to_string(), runner_id.to_string()))
        .lock_exclusive()
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| {
            PostgresError::permission_denied("runner is not attached to the repository")
        })?
        .try_into_domain()
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

async fn ensure_runner_authorized(
    tx: &DatabaseTransaction,
    run: &Run,
    attempt: &RunAttempt,
) -> Result<Runner, PostgresError> {
    let runner = runner_by_id(tx, &attempt.runner_id).await?;
    let grant = grant_by_ids(tx, run.workflow.repository_id(), &attempt.runner_id).await?;
    if !runner.enabled || !grant.is_active() {
        return Err(PostgresError::permission_denied(
            "runner or repository grant is revoked",
        ));
    }
    Ok(runner)
}

async fn save_run(tx: &DatabaseTransaction, run: &Run) -> Result<(), PostgresError> {
    entities::run::Entity::update(
        entities::run::Model::from_domain(run)?
            .into_active_model()
            .reset_all(),
    )
    .exec(tx)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

async fn save_attempt(tx: &DatabaseTransaction, attempt: &RunAttempt) -> Result<(), PostgresError> {
    entities::run_attempt::Entity::update(
        entities::run_attempt::Model::from_domain(attempt)?
            .into_active_model()
            .reset_all(),
    )
    .exec(tx)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

async fn save_runner(tx: &DatabaseTransaction, runner: &Runner) -> Result<(), PostgresError> {
    entities::runner::Entity::update(
        entities::runner::Model::from_domain(runner)?
            .into_active_model()
            .reset_all(),
    )
    .exec(tx)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

fn unique_conflict(error: DbErr, message: &str) -> PostgresError {
    if matches!(
        error.sql_err(),
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
    ) {
        PostgresError::conflict(message)
    } else {
        PostgresError::internal(error)
    }
}
