use super::{
    AdminStore, RunStore, entities,
    run_attempt_persistence::{
        ensure_runner_authorized, locked_attempt_context, locked_run, save_attempt,
        save_attempt_steps, save_run,
    },
};
use crate::error::{PostgresError, PostgresErrorKind};
use scope_domain::runs::{
    cutover::{
        CanaryGeneration, RunnerProtocolCanary, RunnerProtocolCanaryPhase,
        RunnerProtocolCanaryStatus, RunnerProtocolCutover, RunnerProtocolCutoverState,
        validate_runner_protocol_canary_workflow,
    },
    run::{AttemptState, PinnedContainerImage, Run, RunTrigger},
    runner::Runner,
    workflow::WorkflowRevision,
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseTransaction, EntityTrait, Statement, TransactionTrait,
};

const CUTOVER_KEY: &str = "current";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerProtocolCutoverSnapshot {
    pub cutover: RunnerProtocolCutover,
    pub canary_generation: u64,
    pub canaries: Vec<RunnerProtocolCanary>,
}

pub(super) enum DispatchCutover {
    None,
    General,
    Canary(String),
}

impl AdminStore {
    pub async fn runner_protocol_cutover(
        &self,
    ) -> Result<RunnerProtocolCutoverSnapshot, PostgresError> {
        load_snapshot(self.db.as_ref(), false).await
    }

    pub async fn advance_runner_protocol_cutover(
        &self,
        next: RunnerProtocolCutoverState,
        now_unix: u64,
    ) -> Result<RunnerProtocolCutoverSnapshot, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let mut snapshot = load_snapshot(&tx, true).await?;
        let current = snapshot.cutover.state();

        if current != next
            && (current, next)
                != (
                    RunnerProtocolCutoverState::V4Fenced,
                    RunnerProtocolCutoverState::V4Open,
                )
        {
            return Err(PostgresError::conflict(
                "startup migration owns the V3 rewrite; runtime may only open a fenced V4 cutover",
            ));
        }
        if current == RunnerProtocolCutoverState::V4Fenced
            && next == RunnerProtocolCutoverState::V4Open
            && !canary_suite_succeeded(&snapshot)
        {
            return Err(PostgresError::conflict(
                "the current cold-write, warm-read, and evict canary generation must succeed before V4 opens",
            ));
        }

        if snapshot
            .cutover
            .transition(next)
            .map_err(PostgresError::from)?
        {
            save_cutover_state(&tx, next, snapshot.canary_generation, now_unix).await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        snapshot.cutover = RunnerProtocolCutover::restore(next);
        Ok(snapshot)
    }

    pub async fn create_runner_protocol_canary(
        &self,
        phase: RunnerProtocolCanaryPhase,
        runner_id: &str,
        run_id: &str,
        now_unix: u64,
    ) -> Result<RunnerProtocolCutoverSnapshot, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let mut snapshot = load_snapshot(&tx, true).await?;
        if snapshot.cutover.state() != RunnerProtocolCutoverState::V4Fenced {
            return Err(PostgresError::conflict(
                "runner protocol canaries can only be assigned while V4 is fenced",
            ));
        }

        reconcile_abandoned_running_canary(&tx, &mut snapshot, now_unix).await?;

        if let Some(index) = snapshot
            .canaries
            .iter()
            .position(|canary| canary.status() == RunnerProtocolCanaryStatus::Pending)
        {
            let pending = &snapshot.canaries[index];
            if phase != pending.phase() {
                return Err(PostgresError::conflict(format!(
                    "pending runner protocol canary phase is {}",
                    phase_name(pending.phase())
                )));
            }
            ensure_canary_target(&tx, pending.phase(), runner_id, run_id, &snapshot.canaries)
                .await?;
            if pending.run_id() != run_id {
                tx.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "UPDATE scope_runs
                     SET state = 'canceled', cancellation_requested = TRUE,
                         updated_at_unix = $1, completed_at_unix = $1
                     WHERE id = $2 AND state = 'queued' AND current_attempt_id IS NULL",
                    [
                        u64_to_i64(now_unix, "canary reassignment time")?.into(),
                        pending.run_id().into(),
                    ],
                ))
                .await
                .map_err(PostgresError::internal)?;
            }
            let replacement =
                RunnerProtocolCanary::new(pending.generation(), pending.phase(), runner_id, run_id)
                    .map_err(PostgresError::from)?;
            tx.execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_runner_protocol_canaries
                 SET runner_id = $1, run_id = $2, updated_at_unix = $3
                 WHERE generation = $4 AND phase = $5 AND status = 'pending'",
                [
                    runner_id.into(),
                    run_id.into(),
                    u64_to_i64(now_unix, "canary reassignment time")?.into(),
                    u64_to_i64(pending.generation().get(), "canary generation")?.into(),
                    phase_name(pending.phase()).into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
            tx.commit().await.map_err(PostgresError::internal)?;
            snapshot.canaries[index] = replacement;
            return Ok(snapshot);
        }

        let (generation, expected_phase) = next_canary(&snapshot)?;
        if phase != expected_phase {
            return Err(PostgresError::conflict(format!(
                "next runner protocol canary phase is {}",
                phase_name(expected_phase)
            )));
        }
        let identity_canaries = if generation.get() == snapshot.canary_generation {
            snapshot.canaries.as_slice()
        } else {
            &[]
        };
        ensure_canary_target(&tx, phase, runner_id, run_id, identity_canaries).await?;
        let canary = RunnerProtocolCanary::new(generation, phase, runner_id, run_id)
            .map_err(PostgresError::from)?;
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_runner_protocol_canaries (
                generation, phase, runner_id, run_id, status, created_at_unix, updated_at_unix
             ) VALUES ($1, $2, $3, $4, 'pending', $5, $5)",
            [
                u64_to_i64(generation.get(), "canary generation")?.into(),
                phase_name(phase).into(),
                runner_id.into(),
                run_id.into(),
                u64_to_i64(now_unix, "canary creation time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;

        if snapshot.canary_generation != generation.get() {
            snapshot.canary_generation = generation.get();
            snapshot.canaries.clear();
            tx.execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "DELETE FROM scope_runner_protocol_canaries WHERE generation < $1",
                [u64_to_i64(generation.get(), "canary generation")?.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
            save_cutover_state(
                &tx,
                RunnerProtocolCutoverState::V4Fenced,
                snapshot.canary_generation,
                now_unix,
            )
            .await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        snapshot.canaries.push(canary);
        Ok(snapshot)
    }
}

async fn reconcile_abandoned_running_canary(
    tx: &DatabaseTransaction,
    snapshot: &mut RunnerProtocolCutoverSnapshot,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let Some(index) = snapshot
        .canaries
        .iter()
        .position(|canary| canary.status() == RunnerProtocolCanaryStatus::Running)
    else {
        return Ok(());
    };
    let current = &snapshot.canaries[index];
    let mut canary = current_canary_for_run(
        tx,
        snapshot.canary_generation,
        current.runner_id(),
        current.run_id(),
        true,
    )
    .await?;
    let mut run = locked_run(tx, canary.run_id()).await?;

    if let Some(attempt_id) = run.current_attempt_id.clone() {
        let (mut locked_run, mut attempt, mut steps) =
            locked_attempt_context(tx, &attempt_id).await?;
        if attempt.run_id != run.id || attempt.runner_id != canary.runner_id() {
            return Err(PostgresError::internal_message(
                "canary run attempt identity is inconsistent",
            ));
        }
        if !attempt.state.is_terminal() {
            if now_unix < attempt.lease_expires_at_unix {
                return Err(PostgresError::conflict(format!(
                    "the running runner protocol canary attempt is active until unix timestamp {}; retry canary creation after that deadline",
                    attempt.lease_expires_at_unix
                )));
            }
            attempt
                .expire(&mut locked_run, &mut steps, now_unix)
                .map_err(PostgresError::from)?;
            if locked_run.state == scope_domain::runs::run::RunState::Queued {
                locked_run
                    .request_cancellation(now_unix)
                    .map_err(PostgresError::from)?;
            }
            save_attempt(tx, &attempt).await?;
            save_attempt_steps(tx, &steps).await?;
            save_run(tx, &locked_run).await?;
        }
        if attempt.state == AttemptState::Succeeded && now_unix < attempt.token_expires_at_unix {
            return Err(PostgresError::conflict(format!(
                "the successful runner protocol canary can still finalize its cache until unix timestamp {}; retry canary creation after that deadline",
                attempt.token_expires_at_unix
            )));
        }
    } else if !run.state.is_terminal() {
        run.request_cancellation(now_unix)
            .map_err(PostgresError::from)?;
        save_run(tx, &run).await?;
    }

    canary.retire_abandoned().map_err(PostgresError::from)?;
    save_canary_status(tx, &canary, now_unix).await?;
    snapshot.canaries[index] = canary;
    Ok(())
}

impl RunStore {
    pub async fn runner_protocol_cutover(
        &self,
    ) -> Result<RunnerProtocolCutoverSnapshot, PostgresError> {
        load_snapshot(self.db.as_ref(), false).await
    }

    pub async fn finalize_runner_protocol_canary_cache(
        &self,
        attempt_id: &str,
        token_hash: &str,
        succeeded: bool,
        now_unix: u64,
    ) -> Result<RunnerProtocolCanary, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let target = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("run attempt not found"))?;
        let (cutover, generation) = load_cutover(&tx, true).await?;
        let mut canary =
            current_canary_for_run(&tx, generation, &target.runner_id, &target.run_id, true)
                .await?;
        let (run, attempt, _) = locked_attempt_context(&tx, attempt_id).await?;
        ensure_runner_authorized(&tx, &run, &attempt).await?;
        if attempt.token_hash != token_hash
            || run.current_attempt_id.as_deref() != Some(attempt.id.as_str())
        {
            return Err(PostgresError::permission_denied(
                "attempt credentials are invalid",
            ));
        }
        if attempt.state != AttemptState::Succeeded {
            return Err(PostgresError::conflict(
                "cache finalization requires a successful canary attempt",
            ));
        }
        if cutover.state() == RunnerProtocolCutoverState::V4Open
            && succeeded
            && canary.status() == RunnerProtocolCanaryStatus::Succeeded
        {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(canary);
        }
        if cutover.state() != RunnerProtocolCutoverState::V4Fenced {
            return Err(PostgresError::unavailable(
                "cache finalization is only accepted for the active fenced canary",
            ));
        }
        if succeeded {
            canary.succeed().map_err(PostgresError::from)?;
        } else {
            canary.fail().map_err(PostgresError::from)?;
        }
        save_canary_status(&tx, &canary, now_unix).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(canary)
    }
}

pub(super) async fn guard_runner_registration(
    tx: &DatabaseTransaction,
    runner: &Runner,
) -> Result<(), PostgresError> {
    let state = load_cutover(tx, false).await?.0.state();
    if !state.allows_runner_registration(runner.protocol_version) {
        return Err(PostgresError::conflict(format!(
            "runner protocol {} cannot register while cutover is {}",
            runner.protocol_version,
            cutover_state_name(state)
        )));
    }
    Ok(())
}

pub(super) async fn guard_runner_authentication(
    tx: &DatabaseTransaction,
    runner: &Runner,
) -> Result<(), PostgresError> {
    let state = load_cutover(tx, false).await?.0.state();
    if !runner.enabled || !state.allows_runner_authentication(runner.protocol_version) {
        return Err(PostgresError::permission_denied(
            "runner is disabled or incompatible with the active protocol cutover state",
        ));
    }
    Ok(())
}

pub(super) async fn guard_enqueue(
    tx: &DatabaseTransaction,
    run: &Run,
    revision: &WorkflowRevision,
) -> Result<(), PostgresError> {
    let state = load_cutover(tx, false).await?.0.state();
    if state.allows_enqueue() {
        return Ok(());
    }
    let canonical_target =
        run.trigger == RunTrigger::Manual && run.desired_runner == *revision.definition().runner();
    if !state.allows_canary() || canary_candidate_phase(revision).is_none() || !canonical_target {
        return Err(PostgresError::unavailable(format!(
            "only canonical runner protocol canary runs may be created while cutover is {}",
            cutover_state_name(state)
        )));
    }
    Ok(())
}

pub(super) async fn guard_general_run_write(tx: &DatabaseTransaction) -> Result<(), PostgresError> {
    let state = load_cutover(tx, false).await?.0.state();
    if !state.allows_workflow_writes() {
        return Err(PostgresError::unavailable(format!(
            "run mutation is fenced while protocol cutover is {}",
            cutover_state_name(state)
        )));
    }
    Ok(())
}

pub(super) async fn allows_run_retention(tx: &DatabaseTransaction) -> Result<bool, PostgresError> {
    Ok(load_cutover(tx, false)
        .await?
        .0
        .state()
        .allows_workflow_writes())
}

pub(super) async fn dispatch_cutover(
    tx: &DatabaseTransaction,
    runner_id: &str,
    protocol_version: u32,
) -> Result<DispatchCutover, PostgresError> {
    let (cutover, generation) = load_cutover(tx, false).await?;
    let state = cutover.state();
    if state.allows_claim(protocol_version) {
        return Ok(DispatchCutover::General);
    }
    match state {
        state if state.allows_canary() && state.allows_runner_authentication(protocol_version) => {
            let row = tx
                .query_one(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "SELECT run_id FROM scope_runner_protocol_canaries
                     WHERE generation = $1 AND runner_id = $2 AND status = 'pending'",
                    [
                        u64_to_i64(generation, "canary generation")?.into(),
                        runner_id.into(),
                    ],
                ))
                .await
                .map_err(PostgresError::internal)?;
            Ok(match row {
                Some(row) => DispatchCutover::Canary(
                    row.try_get::<String>("", "run_id")
                        .map_err(PostgresError::internal)?,
                ),
                None => DispatchCutover::None,
            })
        }
        _ => Ok(DispatchCutover::None),
    }
}

pub(super) async fn guard_claim(
    tx: &DatabaseTransaction,
    runner_id: &str,
    run_id: &str,
) -> Result<(), PostgresError> {
    let (cutover, _) = load_cutover(tx, false).await?;
    let runner = entities::runner::Entity::find_by_id(runner_id.to_string())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("runner not found"))?
        .try_into_domain()?;
    let state = cutover.state();
    if state.allows_claim(runner.protocol_version) {
        return Ok(());
    }
    match state {
        state
            if state.allows_canary()
                && state.allows_runner_authentication(runner.protocol_version) =>
        {
            // Canary mutations lock the singleton before the canary row. Use
            // the same order so reassignment cannot deadlock a claim.
            let (locked, generation) = load_cutover(tx, true).await?;
            if !locked.state().allows_canary() {
                return Err(PostgresError::unavailable(
                    "runner protocol canary fence opened before claim",
                ));
            }
            let canary = current_canary_for_run(tx, generation, runner_id, run_id, true).await?;
            if canary.status() != RunnerProtocolCanaryStatus::Pending {
                return Err(PostgresError::conflict(
                    "runner protocol canary has already been claimed",
                ));
            }
            Ok(())
        }
        _ => Err(PostgresError::unavailable(format!(
            "run claims are fenced while protocol cutover is {}",
            cutover_state_name(state)
        ))),
    }
}

pub(super) async fn mark_canary_claimed(
    tx: &DatabaseTransaction,
    runner_id: &str,
    run_id: &str,
    now_unix: u64,
) -> Result<Option<RunnerProtocolCanaryPhase>, PostgresError> {
    let (cutover, _) = load_cutover(tx, false).await?;
    if cutover.state() != RunnerProtocolCutoverState::V4Fenced {
        return Ok(None);
    }
    let (cutover, generation) = load_cutover(tx, true).await?;
    if cutover.state() != RunnerProtocolCutoverState::V4Fenced {
        return Ok(None);
    }
    let mut canary = current_canary_for_run(tx, generation, runner_id, run_id, true).await?;
    canary.start().map_err(PostgresError::from)?;
    save_canary_status(tx, &canary, now_unix).await?;
    Ok(Some(canary.phase()))
}

pub(super) async fn guard_attempt_operation(
    tx: &DatabaseTransaction,
    runner_id: &str,
    run_id: &str,
) -> Result<(), PostgresError> {
    let (cutover, generation) = load_cutover(tx, false).await?;
    let runner = entities::runner::Entity::find_by_id(runner_id.to_string())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("runner not found"))?
        .try_into_domain()?;
    // Keep the open-state hot path lock-free. While fenced, serialize the
    // operation with canary reassignment and phase advancement using the same
    // singleton -> canary lock order as claims and terminal updates.
    let (cutover, generation) = if cutover.state() == RunnerProtocolCutoverState::V4Fenced {
        load_cutover(tx, true).await?
    } else {
        (cutover, generation)
    };
    let allowed = cutover
        .state()
        .allows_attempt_operation(runner.protocol_version)
        && match cutover.state() {
            RunnerProtocolCutoverState::V4Fenced => {
                match current_canary_for_run(tx, generation, runner_id, run_id, true).await {
                    Ok(_) => true,
                    Err(error) if error.kind == PostgresErrorKind::Unavailable => false,
                    Err(error) => return Err(error),
                }
            }
            _ => true,
        };
    if !allowed {
        return Err(PostgresError::unavailable(format!(
            "attempt operation is fenced while protocol cutover is {}",
            cutover_state_name(cutover.state())
        )));
    }
    Ok(())
}

pub(super) async fn guard_canary_pinned_image(
    tx: &DatabaseTransaction,
    runner_id: &str,
    run_id: &str,
    revision: &WorkflowRevision,
    image: &PinnedContainerImage,
) -> Result<(), PostgresError> {
    let (cutover, generation) = load_cutover(tx, false).await?;
    if cutover.state() != RunnerProtocolCutoverState::V4Fenced {
        return Ok(());
    }
    let canary = current_canary_for_run(tx, generation, runner_id, run_id, false).await?;
    validate_runner_protocol_canary_workflow(revision.definition(), canary.phase())
        .map_err(PostgresError::from)?;
    if revision.definition().container().image() != image.as_str() {
        return Err(PostgresError::conflict(
            "canary execution must use the workflow's exact digest-pinned image",
        ));
    }
    Ok(())
}

pub(super) async fn record_canary_attempt_terminal(
    tx: &DatabaseTransaction,
    runner_id: &str,
    run_id: &str,
    state: AttemptState,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let (cutover, _) = load_cutover(tx, false).await?;
    if cutover.state() != RunnerProtocolCutoverState::V4Fenced
        || !state.is_terminal()
        || state == AttemptState::Succeeded
    {
        return Ok(());
    }
    let (cutover, generation) = load_cutover(tx, true).await?;
    if cutover.state() != RunnerProtocolCutoverState::V4Fenced {
        return Ok(());
    }
    let mut canary = match current_canary_for_run(tx, generation, runner_id, run_id, true).await {
        Ok(canary) => canary,
        Err(error) if error.kind == PostgresErrorKind::Unavailable => return Ok(()),
        Err(error) => return Err(error),
    };
    canary.fail().map_err(PostgresError::from)?;
    save_canary_status(tx, &canary, now_unix).await
}

async fn load_snapshot<C>(
    conn: &C,
    lock: bool,
) -> Result<RunnerProtocolCutoverSnapshot, PostgresError>
where
    C: ConnectionTrait,
{
    let (cutover, canary_generation) = load_cutover(conn, lock).await?;
    let rows = conn
        .query_all(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT generation, phase, runner_id, run_id, status
             FROM scope_runner_protocol_canaries
             WHERE generation = $1
             ORDER BY CASE phase WHEN 'cold-write' THEN 1 WHEN 'warm-read' THEN 2 ELSE 3 END",
            [u64_to_i64(canary_generation, "canary generation")?.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
    let canaries = rows
        .into_iter()
        .map(|row| canary_from_row(&row))
        .collect::<Result<_, _>>()?;
    Ok(RunnerProtocolCutoverSnapshot {
        cutover,
        canary_generation,
        canaries,
    })
}

async fn load_cutover<C>(
    conn: &C,
    lock: bool,
) -> Result<(RunnerProtocolCutover, u64), PostgresError>
where
    C: ConnectionTrait,
{
    let sql = format!(
        "SELECT state, canary_generation FROM scope_runner_protocol_cutover WHERE key = $1{}",
        if lock { " FOR UPDATE" } else { "" }
    );
    let row = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            sql,
            [CUTOVER_KEY.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("runner protocol cutover row is missing"))?;
    let state = parse_cutover_state(
        &row.try_get::<String>("", "state")
            .map_err(PostgresError::internal)?,
    )?;
    let generation = i64_to_u64(
        row.try_get::<i64>("", "canary_generation")
            .map_err(PostgresError::internal)?,
        "canary generation",
    )?;
    Ok((RunnerProtocolCutover::restore(state), generation))
}

async fn save_cutover_state(
    tx: &DatabaseTransaction,
    state: RunnerProtocolCutoverState,
    generation: u64,
    now_unix: u64,
) -> Result<(), PostgresError> {
    tx.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_runner_protocol_cutover
         SET state = $1, canary_generation = $2, updated_at_unix = $3
         WHERE key = $4",
        [
            cutover_state_name(state).into(),
            u64_to_i64(generation, "canary generation")?.into(),
            u64_to_i64(now_unix, "cutover update time")?.into(),
            CUTOVER_KEY.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

fn next_canary(
    snapshot: &RunnerProtocolCutoverSnapshot,
) -> Result<(CanaryGeneration, RunnerProtocolCanaryPhase), PostgresError> {
    if snapshot.canary_generation == 0 {
        return Ok((
            CanaryGeneration::new(1).map_err(PostgresError::from)?,
            RunnerProtocolCanaryPhase::ColdWrite,
        ));
    }
    let generation =
        CanaryGeneration::new(snapshot.canary_generation).map_err(PostgresError::from)?;
    if snapshot
        .canaries
        .iter()
        .any(|canary| canary.status() == RunnerProtocolCanaryStatus::Failed)
    {
        return Ok((
            generation.next().map_err(PostgresError::from)?,
            RunnerProtocolCanaryPhase::ColdWrite,
        ));
    }
    if snapshot.canaries.iter().any(|canary| {
        matches!(
            canary.status(),
            RunnerProtocolCanaryStatus::Pending | RunnerProtocolCanaryStatus::Running
        )
    }) {
        return Err(PostgresError::conflict(
            "the current runner protocol canary must become terminal before assigning another",
        ));
    }
    let phase = match snapshot.canaries.as_slice() {
        [] => RunnerProtocolCanaryPhase::ColdWrite,
        [cold]
            if cold.phase() == RunnerProtocolCanaryPhase::ColdWrite
                && cold.status() == RunnerProtocolCanaryStatus::Succeeded =>
        {
            RunnerProtocolCanaryPhase::WarmRead
        }
        [cold, warm]
            if cold.status() == RunnerProtocolCanaryStatus::Succeeded
                && warm.phase() == RunnerProtocolCanaryPhase::WarmRead
                && warm.status() == RunnerProtocolCanaryStatus::Succeeded =>
        {
            RunnerProtocolCanaryPhase::Evict
        }
        _ => {
            return Err(PostgresError::conflict(
                "runner protocol canary generation is complete or inconsistent",
            ));
        }
    };
    Ok((generation, phase))
}

fn canary_suite_succeeded(snapshot: &RunnerProtocolCutoverSnapshot) -> bool {
    snapshot.canary_generation > 0
        && snapshot.canaries.len() == 3
        && [
            RunnerProtocolCanaryPhase::ColdWrite,
            RunnerProtocolCanaryPhase::WarmRead,
            RunnerProtocolCanaryPhase::Evict,
        ]
        .into_iter()
        .all(|phase| {
            snapshot.canaries.iter().any(|canary| {
                canary.phase() == phase && canary.status() == RunnerProtocolCanaryStatus::Succeeded
            })
        })
}

async fn ensure_canary_target(
    tx: &DatabaseTransaction,
    phase: RunnerProtocolCanaryPhase,
    runner_id: &str,
    run_id: &str,
    generation_canaries: &[RunnerProtocolCanary],
) -> Result<(), PostgresError> {
    let runner = entities::runner::Entity::find_by_id(runner_id.to_string())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("canary runner not found"))?
        .try_into_domain()?;
    if !runner.supports_dispatch() {
        return Err(PostgresError::conflict(
            "canary runner must be enabled and support protocol V4",
        ));
    }
    let run = entities::run::Entity::find_by_id(run_id.to_string())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("canary run not found"))?
        .try_into_domain()?;
    if run.state != scope_domain::runs::run::RunState::Queued || run.current_attempt_id.is_some() {
        return Err(PostgresError::conflict(
            "canary run must be unclaimed and queued",
        ));
    }
    let grant = entities::runner_grant::Entity::find_by_id((
        run.workflow.repository_id().to_string(),
        runner_id.to_string(),
    ))
    .one(tx)
    .await
    .map_err(PostgresError::internal)?
    .ok_or_else(|| PostgresError::conflict("canary runner is not attached to the run repository"))?
    .try_into_domain()?;
    if !grant.is_active() || !run.desired_runner.matches_name(grant.name.as_str()) {
        return Err(PostgresError::conflict(
            "canary runner grant does not match the queued run",
        ));
    }
    let revision = workflow_revision_for_target(tx, &run).await?;
    validate_runner_protocol_canary_workflow(revision.definition(), phase)
        .map_err(PostgresError::from)?;
    if run.trigger != RunTrigger::Manual || run.desired_runner != *revision.definition().runner() {
        return Err(PostgresError::conflict(
            "canary run must preserve the canonical workflow trigger and exact runner",
        ));
    }

    for previous in generation_canaries
        .iter()
        .filter(|canary| canary.phase() != phase)
    {
        if previous.runner_id() != runner_id {
            return Err(PostgresError::conflict(
                "all canary phases in a generation must use the same runner",
            ));
        }
        let previous_run = entities::run::Entity::find_by_id(previous.run_id().to_string())
            .one(tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("persisted canary run is missing"))?
            .try_into_domain()?;
        let previous_revision = workflow_revision_for_target(tx, &previous_run).await?;
        validate_runner_protocol_canary_workflow(previous_revision.definition(), previous.phase())
            .map_err(PostgresError::from)?;
        if previous_run.workflow.repository_id() != run.workflow.repository_id()
            || previous_revision.definition().container().image()
                != revision.definition().container().image()
            || previous_revision.definition().caches() != revision.definition().caches()
        {
            return Err(PostgresError::conflict(
                "all canary phases in a generation must use one repository, pinned image, and cache identity",
            ));
        }
    }
    Ok(())
}

async fn workflow_revision_for_target(
    tx: &DatabaseTransaction,
    run: &Run,
) -> Result<WorkflowRevision, PostgresError> {
    entities::workflow_revision::Entity::find_by_id(run.workflow_revision_digest.clone())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("canary workflow revision is missing"))?
        .try_into_domain(run.workflow.clone())
}

fn canary_candidate_phase(revision: &WorkflowRevision) -> Option<RunnerProtocolCanaryPhase> {
    [
        RunnerProtocolCanaryPhase::ColdWrite,
        RunnerProtocolCanaryPhase::WarmRead,
        RunnerProtocolCanaryPhase::Evict,
    ]
    .into_iter()
    .find(|phase| validate_runner_protocol_canary_workflow(revision.definition(), *phase).is_ok())
}

async fn current_canary_for_run<C>(
    conn: &C,
    generation: u64,
    runner_id: &str,
    run_id: &str,
    lock: bool,
) -> Result<RunnerProtocolCanary, PostgresError>
where
    C: ConnectionTrait,
{
    let sql = format!(
        "SELECT generation, phase, runner_id, run_id, status
         FROM scope_runner_protocol_canaries
         WHERE generation = $1 AND runner_id = $2 AND run_id = $3{}",
        if lock { " FOR UPDATE" } else { "" }
    );
    let row = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            sql,
            [
                u64_to_i64(generation, "canary generation")?.into(),
                runner_id.into(),
                run_id.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::unavailable("run is not the active protocol V4 canary"))?;
    canary_from_row(&row)
}

async fn save_canary_status(
    tx: &DatabaseTransaction,
    canary: &RunnerProtocolCanary,
    now_unix: u64,
) -> Result<(), PostgresError> {
    tx.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_runner_protocol_canaries
         SET status = $1, updated_at_unix = $2
         WHERE generation = $3 AND phase = $4",
        [
            canary_status_name(canary.status()).into(),
            u64_to_i64(now_unix, "canary update time")?.into(),
            u64_to_i64(canary.generation().get(), "canary generation")?.into(),
            phase_name(canary.phase()).into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

fn canary_from_row(row: &sea_orm::QueryResult) -> Result<RunnerProtocolCanary, PostgresError> {
    RunnerProtocolCanary::restore(
        CanaryGeneration::new(i64_to_u64(
            row.try_get::<i64>("", "generation")
                .map_err(PostgresError::internal)?,
            "canary generation",
        )?)
        .map_err(PostgresError::from)?,
        parse_canary_phase(
            &row.try_get::<String>("", "phase")
                .map_err(PostgresError::internal)?,
        )?,
        row.try_get::<String>("", "runner_id")
            .map_err(PostgresError::internal)?,
        row.try_get::<String>("", "run_id")
            .map_err(PostgresError::internal)?,
        parse_canary_status(
            &row.try_get::<String>("", "status")
                .map_err(PostgresError::internal)?,
        )?,
    )
    .map_err(PostgresError::from)
}

fn parse_cutover_state(value: &str) -> Result<RunnerProtocolCutoverState, PostgresError> {
    match value {
        "v4-fenced" => Ok(RunnerProtocolCutoverState::V4Fenced),
        "v4-open" => Ok(RunnerProtocolCutoverState::V4Open),
        _ => Err(PostgresError::internal_message(
            "invalid runner protocol cutover state",
        )),
    }
}

fn cutover_state_name(state: RunnerProtocolCutoverState) -> &'static str {
    match state {
        RunnerProtocolCutoverState::V4Fenced => "v4-fenced",
        RunnerProtocolCutoverState::V4Open => "v4-open",
    }
}

fn parse_canary_phase(value: &str) -> Result<RunnerProtocolCanaryPhase, PostgresError> {
    match value {
        "cold-write" => Ok(RunnerProtocolCanaryPhase::ColdWrite),
        "warm-read" => Ok(RunnerProtocolCanaryPhase::WarmRead),
        "evict" => Ok(RunnerProtocolCanaryPhase::Evict),
        _ => Err(PostgresError::internal_message(
            "invalid runner protocol canary phase",
        )),
    }
}

fn phase_name(phase: RunnerProtocolCanaryPhase) -> &'static str {
    match phase {
        RunnerProtocolCanaryPhase::ColdWrite => "cold-write",
        RunnerProtocolCanaryPhase::WarmRead => "warm-read",
        RunnerProtocolCanaryPhase::Evict => "evict",
    }
}

fn parse_canary_status(value: &str) -> Result<RunnerProtocolCanaryStatus, PostgresError> {
    match value {
        "pending" => Ok(RunnerProtocolCanaryStatus::Pending),
        "running" => Ok(RunnerProtocolCanaryStatus::Running),
        "succeeded" => Ok(RunnerProtocolCanaryStatus::Succeeded),
        "failed" => Ok(RunnerProtocolCanaryStatus::Failed),
        _ => Err(PostgresError::internal_message(
            "invalid runner protocol canary status",
        )),
    }
}

fn canary_status_name(status: RunnerProtocolCanaryStatus) -> &'static str {
    match status {
        RunnerProtocolCanaryStatus::Pending => "pending",
        RunnerProtocolCanaryStatus::Running => "running",
        RunnerProtocolCanaryStatus::Succeeded => "succeeded",
        RunnerProtocolCanaryStatus::Failed => "failed",
    }
}

fn u64_to_i64(value: u64, label: &str) -> Result<i64, PostgresError> {
    i64::try_from(value).map_err(|_| PostgresError::invalid_input(format!("{label} is too large")))
}

fn i64_to_u64(value: i64, label: &str) -> Result<u64, PostgresError> {
    u64::try_from(value)
        .map_err(|_| PostgresError::internal_message(format!("{label} is negative")))
}
