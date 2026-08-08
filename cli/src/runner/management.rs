use super::{
    RunnerConfig, cache,
    config::{load_runner_config_from, runner_cache_root, runner_config_path, store_runner_config},
    doctor_local, load_runner_config, runner_client, runner_poll,
    systemd::{install_systemd_service, print_linger_status},
    unix_now,
};
use crate::{
    api::{
        api_url, attach_runner_repository, detach_runner_repository, get_repo, get_runner,
        register_runner, upgrade_runner_registration,
    },
    login::session_from_cache_or_device,
};
use anyhow::bail;
use scope_api_contract::{RegisterRunnerRequest, RunnerResponse, UpgradeRunnerRegistrationRequest};
use scope_domain::runs::runner::{
    RUNNER_PROTOCOL_VERSION, RunnerCapabilities, RunnerMaxConcurrentJobs,
};

pub fn install(
    name: &str,
    repository: &str,
    requested_max_concurrent_jobs: Option<u8>,
) -> anyhow::Result<()> {
    let (owner, repo) = parse_repository(repository)?;
    let api_url = api_url();
    let client = runner_client()?;
    let session = session_from_cache_or_device(&client, &api_url)?;
    let config_path = runner_config_path()?;
    if config_path.exists() {
        let mut config = load_runner_config_from(&config_path)?;
        if config.api_url != api_url || config.name != name {
            bail!(
                "this machine is already configured as runner {} for {}; remove {} before replacing it",
                config.name,
                config.api_url,
                config_path.display()
            );
        }
        let max_concurrent_jobs = requested_max_concurrent_jobs
            .map(RunnerMaxConcurrentJobs::new)
            .transpose()?
            .unwrap_or(config.max_concurrent_jobs);
        doctor_local(true, max_concurrent_jobs)?;
        let cache_root = runner_cache_root(config.cache_root.as_deref())?;
        cache::initialize(&cache_root)?;
        let upgraded = upgrade_runner_registration(
            &client,
            &api_url,
            &session.token,
            &config.runner_id,
            &UpgradeRunnerRegistrationRequest {
                version: env!("CARGO_PKG_VERSION").to_string(),
                protocol_version: RUNNER_PROTOCOL_VERSION,
                capabilities: RunnerCapabilities::v1(),
                max_concurrent_jobs,
            },
        )?;
        config.secret = upgraded.secret;
        config.max_concurrent_jobs = max_concurrent_jobs;
        config.cache_root = Some(cache_root);
        store_runner_config(&config_path, &config)?;
        let runner = upgraded.runner;
        let repository_id = get_repo(&client, &api_url, &session.token, owner, repo)?.id;
        if !runner
            .grants
            .iter()
            .any(|grant| grant.active && grant.repository_id == repository_id)
        {
            attach_runner_repository(
                &client,
                &api_url,
                &session.token,
                &config.runner_id,
                owner,
                repo,
                name,
            )?;
        }
        install_systemd_service(&config_path)?;
        println!("✓ Existing runner configuration restored");
        println!("✓ systemd user service installed");
        print_linger_status();
        return Ok(());
    }
    let max_concurrent_jobs =
        RunnerMaxConcurrentJobs::new(requested_max_concurrent_jobs.unwrap_or(1))?;
    doctor_local(true, max_concurrent_jobs)?;
    let requested_cache_root = runner_cache_root(None)?;
    cache::initialize(&requested_cache_root)?;
    let registered = register_runner(
        &client,
        &api_url,
        &session.token,
        &RegisterRunnerRequest {
            owner: owner.to_string(),
            repo: repo.to_string(),
            name: name.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: RUNNER_PROTOCOL_VERSION,
            capabilities: RunnerCapabilities::v1(),
            max_concurrent_jobs,
        },
    )?;
    let config = RunnerConfig {
        api_url,
        runner_id: registered.runner.id,
        name: name.to_string(),
        secret: registered.secret,
        max_concurrent_jobs,
        cache_root: Some(requested_cache_root),
    };
    if let Err(error) = store_runner_config(&config_path, &config) {
        let _ =
            crate::api::delete_runner(&client, &config.api_url, &session.token, &config.runner_id);
        return Err(error);
    }
    install_systemd_service(&config_path)?;
    println!("✓ Runner secret stored with mode 0600");
    println!("✓ Docker available and test container completed");
    println!("✓ systemd user service installed");
    print_linger_status();
    println!("✓ {name} is registered; the service is starting");
    Ok(())
}

pub fn status() -> anyhow::Result<()> {
    let config = load_runner_config()?;
    let client = runner_client()?;
    let session = session_from_cache_or_device(&client, &config.api_url)?;
    let runner = get_runner(&client, &config.api_url, &session.token, &config.runner_id)?;
    print_runner_status(&config.name, &runner);
    Ok(())
}

pub fn add_repository(repository: &str) -> anyhow::Result<()> {
    let config = load_runner_config()?;
    let (owner, repo) = parse_repository(repository)?;
    let client = runner_client()?;
    let session = session_from_cache_or_device(&client, &config.api_url)?;
    attach_runner_repository(
        &client,
        &config.api_url,
        &session.token,
        &config.runner_id,
        owner,
        repo,
        &config.name,
    )?;
    println!("✓ Repository attached");
    Ok(())
}

pub fn remove_repository(repository: &str) -> anyhow::Result<()> {
    let config = load_runner_config()?;
    let (owner, repo) = parse_repository(repository)?;
    let client = runner_client()?;
    let session = session_from_cache_or_device(&client, &config.api_url)?;
    detach_runner_repository(
        &client,
        &config.api_url,
        &session.token,
        &config.runner_id,
        owner,
        repo,
    )?;
    println!("✓ Repository access revoked");
    Ok(())
}

pub fn doctor() -> anyhow::Result<()> {
    let config = load_runner_config().ok();
    let max_concurrent_jobs = config.as_ref().map_or_else(
        || RunnerMaxConcurrentJobs::new(1),
        |config| Ok(config.max_concurrent_jobs),
    )?;
    let (capabilities, limits) = doctor_local(true, max_concurrent_jobs)?;
    if let Some(config) = config {
        cache::doctor(&config)?;
        println!(
            "✓ live resources per slot ({} MiB memory, {:.3} CPU, {} PIDs across {} slot(s))",
            limits.memory_bytes / (1024 * 1024),
            limits.cpu_millis as f64 / 1000.0,
            limits.pids,
            max_concurrent_jobs.get(),
        );
        let client = runner_client()?;
        runner_poll(&client, &config.api_url, &config.secret)?;
        println!("✓ Scope API");
    }
    println!(
        "✓ Docker (writable-layer quotas {})",
        if capabilities.storage_quota_supported {
            "enabled"
        } else {
            "unavailable; best-effort storage guard enabled"
        }
    );
    println!("✓ transient disk");
    println!("✓ cgroups");
    println!("✓ systemd user service");
    Ok(())
}

pub fn list_caches() -> anyhow::Result<()> {
    cache::list(&load_runner_config()?)
}

pub fn prune_caches(all: bool) -> anyhow::Result<()> {
    cache::prune(&load_runner_config()?, all)
}

pub(super) fn print_runner_status(name: &str, runner: &RunnerResponse) {
    let online = runner
        .last_seen_at_unix
        .and_then(|last_seen| unix_now().checked_sub(last_seen))
        .is_some_and(|age| age <= 90);
    println!(
        "{} · {} · {} · {} slot(s)",
        name,
        if online { "online" } else { "offline" },
        if runner.enabled {
            "enabled"
        } else {
            "disabled"
        },
        runner.max_concurrent_jobs.get(),
    );
    for grant in runner.grants.iter().filter(|grant| grant.active) {
        println!("  {} as {}", grant.repository_id, grant.name);
    }
}

pub(super) fn parse_repository(repository: &str) -> anyhow::Result<(&str, &str)> {
    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or_default();
    let repo = parts.next().unwrap_or_default();
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        bail!("expected repository as owner/repo");
    }
    Ok((owner, repo))
}
