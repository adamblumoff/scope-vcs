use crate::error::PostgresError;
use scope_domain::runs::cutover::{
    CanaryGeneration, RunnerProtocolCanary, RunnerProtocolCanaryPhase, RunnerProtocolCanaryStatus,
    RunnerProtocolCutoverState,
};

pub(super) fn canary_from_row(
    row: &sea_orm::QueryResult,
) -> Result<RunnerProtocolCanary, PostgresError> {
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

pub(super) fn parse_cutover_state(
    value: &str,
) -> Result<RunnerProtocolCutoverState, PostgresError> {
    match value {
        "v8-fenced" => Ok(RunnerProtocolCutoverState::V8Fenced),
        "v8-open" => Ok(RunnerProtocolCutoverState::V8Open),
        _ => Err(PostgresError::internal_message(
            "invalid runner protocol cutover state",
        )),
    }
}

pub(super) fn cutover_state_name(state: RunnerProtocolCutoverState) -> &'static str {
    match state {
        RunnerProtocolCutoverState::V8Fenced => "v8-fenced",
        RunnerProtocolCutoverState::V8Open => "v8-open",
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

pub(super) fn phase_name(phase: RunnerProtocolCanaryPhase) -> &'static str {
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

pub(super) fn canary_status_name(status: RunnerProtocolCanaryStatus) -> &'static str {
    match status {
        RunnerProtocolCanaryStatus::Pending => "pending",
        RunnerProtocolCanaryStatus::Running => "running",
        RunnerProtocolCanaryStatus::Succeeded => "succeeded",
        RunnerProtocolCanaryStatus::Failed => "failed",
    }
}

pub(super) fn u64_to_i64(value: u64, label: &str) -> Result<i64, PostgresError> {
    i64::try_from(value).map_err(|_| PostgresError::invalid_input(format!("{label} is too large")))
}

pub(super) fn i64_to_u64(value: i64, label: &str) -> Result<u64, PostgresError> {
    u64::try_from(value)
        .map_err(|_| PostgresError::internal_message(format!("{label} is negative")))
}
