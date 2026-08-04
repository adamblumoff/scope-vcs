use super::{RunnerProtocolCutoverSnapshot, current_canary_for_run, save_canary_status};
use crate::{
    db::run_attempt_persistence::{
        locked_attempt_context, locked_run, save_attempt, save_attempt_steps, save_run,
    },
    error::PostgresError,
};
use scope_domain::runs::{
    cutover::{RunnerProtocolCanaryStatus, RunnerProtocolCutoverState},
    run::{AttemptState, RunState},
};
use sea_orm::DatabaseTransaction;

pub(super) async fn reconcile_abandoned_running_canary(
    tx: &DatabaseTransaction,
    snapshot: &mut RunnerProtocolCutoverSnapshot,
    now_unix: u64,
) -> Result<(), PostgresError> {
    debug_assert_eq!(
        snapshot.cutover.state(),
        RunnerProtocolCutoverState::V4Fenced
    );
    let Some(index) = snapshot
        .canaries
        .iter()
        .position(|canary| canary.status() == RunnerProtocolCanaryStatus::Running)
    else {
        cancel_failed_canary_run(tx, snapshot, now_unix).await?;
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
            save_attempt(tx, &attempt).await?;
            save_attempt_steps(tx, &steps).await?;
        }
        if attempt.state == AttemptState::Succeeded && now_unix < attempt.token_expires_at_unix {
            return Err(PostgresError::conflict(format!(
                "the successful runner protocol canary can still finalize its cache until unix timestamp {}; retry canary creation after that deadline",
                attempt.token_expires_at_unix
            )));
        }
        if locked_run.state == RunState::Queued {
            locked_run
                .request_cancellation(now_unix)
                .map_err(PostgresError::from)?;
        }
        save_run(tx, &locked_run).await?;
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

async fn cancel_failed_canary_run(
    tx: &DatabaseTransaction,
    snapshot: &RunnerProtocolCutoverSnapshot,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let Some(failed) = snapshot
        .canaries
        .iter()
        .find(|canary| canary.status() == RunnerProtocolCanaryStatus::Failed)
    else {
        return Ok(());
    };
    let mut run = locked_run(tx, failed.run_id()).await?;
    if !run.state.is_terminal() {
        run.request_cancellation(now_unix)
            .map_err(PostgresError::from)?;
        save_run(tx, &run).await?;
    }
    Ok(())
}
