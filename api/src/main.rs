use anyhow::Context;
use api::{AppState, router};
use std::{
    net::{Ipv6Addr, SocketAddr},
    time::Duration,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const RUNTIME_TELEMETRY_INTERVAL_SECS_ENV: &str = "SCOPE_RUNTIME_TELEMETRY_INTERVAL_SECS";

fn main() -> anyhow::Result<()> {
    scope_git_process::install_pid1_reaper_if_needed()?;
    run()
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "api=info,scope_postgres=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    start_runtime_telemetry();

    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
    let state = app_state_from_env().await?;

    serve(addr, state).await
}

fn start_runtime_telemetry() {
    let Some(interval) = std::env::var(RUNTIME_TELEMETRY_INTERVAL_SECS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
    else {
        return;
    };
    tokio::spawn(async move {
        loop {
            let snapshot = scope_git_process::current_process_snapshot();
            tracing::info!(
                process_id = snapshot.process_id,
                parent_process_id = snapshot.parent_process_id.unwrap_or(0),
                threads = snapshot.threads.unwrap_or(0),
                open_file_descriptors = snapshot.open_file_descriptors.unwrap_or(0),
                child_processes = snapshot.child_processes.unwrap_or(0),
                zombie_child_processes = snapshot.zombie_child_processes.unwrap_or(0),
                cgroup_pids_current = snapshot.cgroup_pids_current.unwrap_or(0),
                cgroup_pids_max = snapshot.cgroup_pids_max.unwrap_or(0),
                cgroup_pids_unlimited = snapshot.cgroup_pids_unlimited,
                "runtime process snapshot"
            );
            tokio::time::sleep(interval).await;
        }
    });
}

async fn app_state_from_env() -> anyhow::Result<AppState> {
    #[cfg(feature = "local-dev")]
    {
        if api::dev::is_local_dev_env() {
            return api::dev::app_state_from_env().await;
        }
    }

    #[cfg(not(feature = "local-dev"))]
    {
        if std::env::var("SCOPE_ENV").ok().as_deref() == Some("local") {
            anyhow::bail!("SCOPE_ENV=local requires running the API with --features local-dev");
        }
    }

    AppState::from_env().await
}

async fn serve(addr: SocketAddr, state: AppState) -> anyhow::Result<()> {
    let app = router(state);
    tracing::info!(%addr, "starting api");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding server on {addr}"))?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving api")?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
