use super::{load_runner_config, management, runner_client};
use crate::{
    api::{
        advance_runner_protocol_cutover, create_runner_protocol_canary, get_runner_protocol_cutover,
    },
    run,
};
use anyhow::{Context, bail};
use scope_api_contract::{
    AdvanceRunnerProtocolCutoverRequest, CreateRunnerProtocolCanaryRequest,
    RunnerProtocolCutoverResponse,
};
use scope_domain::runs::cutover::{
    RunnerProtocolCanaryPhase, RunnerProtocolCanaryStatus, RunnerProtocolCutoverState,
};

const OPERATOR_TOKEN_ENV: &str = "SCOPE_OPERATOR_TOKEN";

pub fn recover(
    name: &str,
    repository: &str,
    max_concurrent_jobs: Option<u8>,
) -> anyhow::Result<()> {
    management::install(name, repository, max_concurrent_jobs)?;
    let config = load_runner_config()?;
    let operator_token = std::env::var(OPERATOR_TOKEN_ENV)
        .with_context(|| format!("{OPERATOR_TOKEN_ENV} is required to operate the cutover"))?;
    let client = runner_client()?;

    loop {
        let snapshot = get_runner_protocol_cutover(&client, &config.api_url, &operator_token)?;
        if snapshot.state == RunnerProtocolCutoverState::V6Open {
            println!("✓ Runner protocol V6 is open");
            return Ok(());
        }

        if let Some(canary) = snapshot.canaries.iter().find(|canary| {
            matches!(
                canary.status,
                RunnerProtocolCanaryStatus::Pending | RunnerProtocolCanaryStatus::Running
            )
        }) {
            if canary.runner_id != config.runner_id {
                bail!(
                    "runner protocol canary {} is already assigned to another runner",
                    canary.run_id
                );
            }
            println!(
                "Resuming {} canary {}",
                phase_label(canary.phase),
                canary.run_id
            );
            run::watch(&canary.run_id, None)?;
            continue;
        }

        let Some(phase) = next_phase(&snapshot) else {
            let opened = advance_runner_protocol_cutover(
                &client,
                &config.api_url,
                &operator_token,
                &AdvanceRunnerProtocolCutoverRequest {
                    state: RunnerProtocolCutoverState::V6Open,
                },
            )?;
            if opened.state != RunnerProtocolCutoverState::V6Open {
                bail!("runner protocol cutover did not open");
            }
            println!("✓ Runner protocol V6 is open");
            return Ok(());
        };

        let queued = run::queue(workflow_name(phase), Some(name), None)?;
        println!("Queued {} canary", phase_label(phase));
        println!("Run ID: {}", queued.id());
        create_runner_protocol_canary(
            &client,
            &config.api_url,
            &operator_token,
            &CreateRunnerProtocolCanaryRequest {
                runner_id: config.runner_id.clone(),
                run_id: queued.id().to_string(),
                phase,
            },
        )?;
        run::watch_queued(&queued)?;
    }
}

fn next_phase(snapshot: &RunnerProtocolCutoverResponse) -> Option<RunnerProtocolCanaryPhase> {
    if snapshot
        .canaries
        .iter()
        .any(|canary| canary.status == RunnerProtocolCanaryStatus::Failed)
    {
        return Some(RunnerProtocolCanaryPhase::ColdWrite);
    }
    [
        RunnerProtocolCanaryPhase::ColdWrite,
        RunnerProtocolCanaryPhase::WarmRead,
        RunnerProtocolCanaryPhase::Evict,
    ]
    .into_iter()
    .find(|phase| {
        !snapshot.canaries.iter().any(|canary| {
            canary.phase == *phase && canary.status == RunnerProtocolCanaryStatus::Succeeded
        })
    })
}

fn workflow_name(phase: RunnerProtocolCanaryPhase) -> &'static str {
    match phase {
        RunnerProtocolCanaryPhase::ColdWrite => "v6-canary-cold-write",
        RunnerProtocolCanaryPhase::WarmRead => "v6-canary-warm-read",
        RunnerProtocolCanaryPhase::Evict => "v6-canary-evict",
    }
}

fn phase_label(phase: RunnerProtocolCanaryPhase) -> &'static str {
    match phase {
        RunnerProtocolCanaryPhase::ColdWrite => "cold-write",
        RunnerProtocolCanaryPhase::WarmRead => "warm-read",
        RunnerProtocolCanaryPhase::Evict => "eviction",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_api_contract::RunnerProtocolCanaryResponse;

    fn snapshot(
        canaries: Vec<(RunnerProtocolCanaryPhase, RunnerProtocolCanaryStatus)>,
    ) -> RunnerProtocolCutoverResponse {
        RunnerProtocolCutoverResponse {
            state: RunnerProtocolCutoverState::V6Fenced,
            generation: 1,
            enabled_runner_count: 1,
            canaries: canaries
                .into_iter()
                .map(|(phase, status)| RunnerProtocolCanaryResponse {
                    generation: 1,
                    phase,
                    runner_id: "runner-1".to_string(),
                    run_id: "run-1".to_string(),
                    status,
                })
                .collect(),
        }
    }

    #[test]
    fn cutover_resumes_at_the_first_unfinished_phase() {
        assert_eq!(
            next_phase(&snapshot(Vec::new())),
            Some(RunnerProtocolCanaryPhase::ColdWrite)
        );
        assert_eq!(
            next_phase(&snapshot(vec![(
                RunnerProtocolCanaryPhase::ColdWrite,
                RunnerProtocolCanaryStatus::Succeeded,
            )])),
            Some(RunnerProtocolCanaryPhase::WarmRead)
        );
        assert_eq!(
            next_phase(&snapshot(vec![
                (
                    RunnerProtocolCanaryPhase::ColdWrite,
                    RunnerProtocolCanaryStatus::Succeeded,
                ),
                (
                    RunnerProtocolCanaryPhase::WarmRead,
                    RunnerProtocolCanaryStatus::Succeeded,
                ),
                (
                    RunnerProtocolCanaryPhase::Evict,
                    RunnerProtocolCanaryStatus::Succeeded,
                ),
            ])),
            None
        );
    }

    #[test]
    fn failed_generation_restarts_from_cold_write() {
        assert_eq!(
            next_phase(&snapshot(vec![
                (
                    RunnerProtocolCanaryPhase::ColdWrite,
                    RunnerProtocolCanaryStatus::Succeeded,
                ),
                (
                    RunnerProtocolCanaryPhase::WarmRead,
                    RunnerProtocolCanaryStatus::Failed,
                ),
            ])),
            Some(RunnerProtocolCanaryPhase::ColdWrite)
        );
    }
}
