mod api;
mod cache;
mod checkout;
mod execute;
mod settings;

use anyhow::Context as _;
use settings::RuntimeSettings;

fn main() -> anyhow::Result<()> {
    let settings = RuntimeSettings::from_env()?;
    let client = api::RuntimeClient::new(&settings)?;
    let claim = client.claim(&settings.bootstrap_token)?;
    let setup_heartbeat = api::RuntimeHeartbeat::start(client.clone());
    let setup_result = setup(&settings, &client, &claim);
    let setup_heartbeat_result = setup_heartbeat.finish();
    match setup_heartbeat_result {
        Ok(true) => {
            client.complete_canceled(false)?;
            return Ok(());
        }
        Ok(false) => {}
        Err(error) => return Err(error),
    }
    let (workspace, caches) = match setup_result {
        Ok(value) => value,
        Err(error) => {
            let message = format!("{error:#}");
            let _ = client.complete_setup_failure(&message);
            return Err(error);
        }
    };
    let execution =
        execute::run_steps(client.clone(), &claim.job.definition, &workspace).map_err(|error| {
            eprintln!("runtime execution transport failed: {error:#}");
            error
        })?;
    let logs_truncated = match execution {
        execute::ExecutionOutcome::Succeeded { logs_truncated } => logs_truncated,
        execute::ExecutionOutcome::Terminal => return Ok(()),
    };
    let finalization_heartbeat = api::RuntimeHeartbeat::start(client.clone());
    for finalization in cache::save_caches(&client, &caches) {
        if let cache::CacheFinalizationOutcome::Skipped { reason, message } = finalization.outcome {
            eprintln!(
                "runtime cache finalization skipped for {} ({reason:?}): {message}",
                finalization.identity_digest
            );
        }
    }
    let cancellation_requested =
        finalization_heartbeat.finish()? || client.heartbeat()?.cancellation_requested;
    if cancellation_requested {
        client.complete_canceled(logs_truncated)
    } else {
        client.complete_succeeded(logs_truncated)
    }
}

fn setup(
    settings: &RuntimeSettings,
    client: &api::RuntimeClient,
    claim: &scope_api_contract::ClaimRuntimeResponse,
) -> anyhow::Result<(std::path::PathBuf, Vec<cache::PreparedCache>)> {
    let work = settings.prepare_work_directory()?;
    let bundle = work.join("source.bundle");
    client.download_source(&claim.job.source_digest, &bundle)?;
    let workspace = work.join("workspace");
    checkout::checkout_exact_commit(&bundle, &workspace, &claim.job.git_oid)?;
    std::env::set_current_dir(&workspace).context("enter run workspace")?;
    let caches = cache::prepare_caches(client, &claim.job)?;
    Ok((workspace, caches))
}
