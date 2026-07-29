use super::{RunStore, entities};
use crate::error::PostgresError;
use scope_domain::runs::{
    run::{AttemptConclusion, Run, RunAttempt},
    runner::{Runner, RunnerGrant},
    workflow::WorkflowRevision,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseTransaction, DbErr, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, TransactionTrait, TryInsertResult,
    sea_query::OnConflict,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchClaim {
    pub run: Run,
    pub attempt: RunAttempt,
    pub workflow_revision: WorkflowRevision,
}

impl RunStore {
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
        let stored = if !matches!(result, TryInsertResult::Inserted(_)) {
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
        if !runner.supports_v1_dispatch() {
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
