use super::{
    AdminStore, RunStore, entities,
    run_attempt_persistence::{
        ensure_runner_authorized, locked_attempt_context, locked_jobs, locked_run, save_jobs,
        save_run,
    },
};
use crate::error::{PostgresError, PostgresErrorKind};
use scope_domain::runs::{
    cutover::{
        CanaryGeneration, RunnerProtocolCanary, RunnerProtocolCanaryPhase,
        RunnerProtocolCanaryStatus, RunnerProtocolCutover, RunnerProtocolCutoverState,
        validate_runner_protocol_canary_workflow,
    },
    job::request_run_cancellation,
    run::{AttemptState, PinnedContainerImage, Run, RunTrigger},
    runner::{RUNNER_PROTOCOL_VERSION, Runner},
    workflow::{WorkflowJob, WorkflowRevision},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseTransaction, EntityTrait, QueryFilter,
    Statement, TransactionTrait,
};

mod codec;
mod recovery;
use codec::{
    canary_from_row, canary_status_name, cutover_state_name, i64_to_u64, parse_cutover_state,
    phase_name, u64_to_i64,
};
use recovery::reconcile_abandoned_running_canary;

const CUTOVER_KEY: &str = "current";

fn canary_job(revision: &WorkflowRevision) -> Result<&WorkflowJob, PostgresError> {
    revision
        .definition()
        .only_job()
        .ok_or_else(|| PostgresError::conflict("canary workflow must contain exactly one job"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerProtocolCutoverSnapshot {
    pub cutover: RunnerProtocolCutover,
    pub canary_generation: u64,
    pub enabled_runner_count: u64,
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
                    RunnerProtocolCutoverState::V8Fenced,
                    RunnerProtocolCutoverState::V8Open,
                )
        {
            return Err(PostgresError::conflict(
                "startup migration owns the protocol rewrite; runtime may only open a fenced V8 cutover",
            ));
        }
        if current == RunnerProtocolCutoverState::V8Fenced
            && next == RunnerProtocolCutoverState::V8Open
            && !canary_suite_succeeded(&snapshot)
        {
            return Err(PostgresError::conflict(
                "the current cold-write, warm-read, and evict canary generation must succeed before V8 opens",
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
        if snapshot.cutover.state() != RunnerProtocolCutoverState::V8Fenced {
            return Err(PostgresError::conflict(
                "runner protocol canaries can only be assigned while V8 is fenced",
            ));
        }

        let retries_running_canary = snapshot.canaries.iter().any(|canary| {
            canary.status() == RunnerProtocolCanaryStatus::Running
                && canary.phase() == phase
                && canary.runner_id() == runner_id
                && canary.run_id() == run_id
        });
        reconcile_abandoned_running_canary(&tx, &mut snapshot, retries_running_canary, now_unix)
            .await?;
        if retries_running_canary {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(snapshot);
        }

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
                cancel_unclaimed_canary_run(&tx, pending.run_id(), now_unix).await?;
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
                RunnerProtocolCutoverState::V8Fenced,
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

async fn cancel_unclaimed_canary_run(
    tx: &DatabaseTransaction,
    run_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let mut jobs = locked_jobs(tx, run_id).await?;
    let mut run = locked_run(tx, run_id).await?;
    if jobs.iter().any(|job| job.current_attempt_id.is_some()) {
        return Err(PostgresError::conflict(
            "pending canary run was claimed before reassignment",
        ));
    }
    request_run_cancellation(&mut run, &mut jobs, now_unix).map_err(PostgresError::from)?;
    save_jobs(tx, &jobs).await?;
    save_run(tx, &run).await
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
        let (run, job, attempt, _) = locked_attempt_context(&tx, attempt_id).await?;
        ensure_runner_authorized(&tx, &run, &attempt).await?;
        if attempt.token_hash != token_hash || attempt.job_key != job.key {
            return Err(PostgresError::permission_denied(
                "attempt credentials are invalid",
            ));
        }
        if attempt.state != AttemptState::Succeeded {
            return Err(PostgresError::conflict(
                "cache finalization requires a successful canary attempt",
            ));
        }
        if cutover.state() == RunnerProtocolCutoverState::V8Open
            && succeeded
            && canary.status() == RunnerProtocolCanaryStatus::Succeeded
        {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(canary);
        }
        if cutover.state() != RunnerProtocolCutoverState::V8Fenced {
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
    // This exception is also the bootstrap path after a populated database is
    // migrated into a new protocol fence: ordinary runs remain blocked while
    // an operator enqueues the canonical canary suite for the upgraded runner.
    if !state.allows_canary()
        || run.trigger != RunTrigger::Manual
        || canary_candidate_phase(revision).is_none()
    {
        return Err(PostgresError::unavailable(format!(
            "only canonical runner protocol canary runs may be created while cutover is {}",
            cutover_state_name(state)
        )));
    }
    let canonical_runner = canary_job(revision)?.runner();
    if run
        .runner_override
        .as_ref()
        .is_some_and(|runner| runner != canonical_runner)
    {
        return Err(PostgresError::unavailable(format!(
            "only canonical runner protocol canary runs may be created while cutover is {}",
            cutover_state_name(state)
        )));
    }
    Ok(())
}

pub(super) async fn guard_push_trigger_enqueue(
    tx: &DatabaseTransaction,
) -> Result<(), PostgresError> {
    let state = load_cutover(tx, false).await?.0.state();
    if state.allows_enqueue() {
        return Ok(());
    }
    Err(PostgresError::unavailable(format!(
        "push-triggered workflows are blocked while runner protocol cutover is {}; upgrade a runner and complete the canary suite",
        cutover_state_name(state)
    )))
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
    if cutover.state() != RunnerProtocolCutoverState::V8Fenced {
        return Ok(None);
    }
    let (cutover, generation) = load_cutover(tx, true).await?;
    if cutover.state() != RunnerProtocolCutoverState::V8Fenced {
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
    let (cutover, generation) = if cutover.state() == RunnerProtocolCutoverState::V8Fenced {
        load_cutover(tx, true).await?
    } else {
        (cutover, generation)
    };
    let allowed = cutover
        .state()
        .allows_attempt_operation(runner.protocol_version)
        && match cutover.state() {
            RunnerProtocolCutoverState::V8Fenced => {
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
    if cutover.state() != RunnerProtocolCutoverState::V8Fenced {
        return Ok(());
    }
    let canary = current_canary_for_run(tx, generation, runner_id, run_id, false).await?;
    validate_runner_protocol_canary_workflow(revision.definition(), canary.phase())
        .map_err(PostgresError::from)?;
    if canary_job(revision)?.container().image() != image.as_str() {
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
    if cutover.state() != RunnerProtocolCutoverState::V8Fenced
        || !state.is_terminal()
        || state == AttemptState::Succeeded
    {
        return Ok(());
    }
    let (cutover, generation) = load_cutover(tx, true).await?;
    if cutover.state() != RunnerProtocolCutoverState::V8Fenced {
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
    let enabled_runner_count = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM scope_runners
             WHERE enabled = TRUE AND protocol_version = $1",
            [i64::from(RUNNER_PROTOCOL_VERSION).into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("runner count is missing"))?
        .try_get::<i64>("", "count")
        .map_err(PostgresError::internal)
        .and_then(|count| i64_to_u64(count, "enabled runner count"))?;
    Ok(RunnerProtocolCutoverSnapshot {
        cutover,
        canary_generation,
        enabled_runner_count,
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
            "canary runner must be enabled and support protocol V8",
        ));
    }
    let run = entities::run::Entity::find_by_id(run_id.to_string())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("canary run not found"))?
        .try_into_domain()?;
    let job = entities::run_job::Entity::find()
        .filter(entities::run_job::Column::RunId.eq(&run.id))
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::conflict("canary run job is missing"))?
        .try_into_domain()?;
    if run.state != scope_domain::runs::run::RunState::Queued || job.current_attempt_id.is_some() {
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
    if !grant.is_active() || !job.desired_runner.matches_name(grant.name.as_str()) {
        return Err(PostgresError::conflict(
            "canary runner grant does not match the queued run",
        ));
    }
    let revision = workflow_revision_for_target(tx, &run).await?;
    validate_runner_protocol_canary_workflow(revision.definition(), phase)
        .map_err(PostgresError::from)?;
    if run.trigger != RunTrigger::Manual || job.desired_runner != *canary_job(&revision)?.runner() {
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
        let previous_job = canary_job(&previous_revision)?;
        let job = canary_job(&revision)?;
        if previous_run.workflow.repository_id() != run.workflow.repository_id()
            || previous_job.container().image() != job.container().image()
            || previous_job.caches() != job.caches()
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
        .ok_or_else(|| PostgresError::unavailable("run is not the active protocol V8 canary"))?;
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
