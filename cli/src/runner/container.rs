use super::{RunnerConfig, command_success};
use anyhow::{Context, bail};
use scope_api_contract::ClaimRunResponse;
use std::{env, path::Path, process::Command, thread, time::Duration};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const CONTAINER_MEMORY: &str = "4g";
const CONTAINER_CPUS: &str = "2";
const CONTAINER_PIDS: &str = "512";
const CONTAINER_STORAGE: &str = "20G";

#[derive(Clone, Copy, Debug)]
pub(super) struct DockerCapabilities {
    pub(super) storage_quota_supported: bool,
}

pub(super) fn doctor_local(run_container: bool) -> anyhow::Result<DockerCapabilities> {
    if env::consts::OS != "linux" || env::consts::ARCH != "x86_64" {
        bail!("V1 runners require Linux on amd64");
    }
    command_success(Command::new("docker").args(["info"]), "connect to Docker")?;
    let mut storage_quota_supported = false;
    if run_container {
        let mut command = Command::new("docker");
        command.args(["run", "--rm"]);
        apply_container_limits(&mut command, false);
        command.args(["alpine:3.20", "true"]);
        command_success(&mut command, "run bounded Docker test container")?;

        let mut quota_test = Command::new("docker");
        quota_test.args(["run", "--rm"]);
        apply_container_limits(&mut quota_test, true);
        quota_test.args(["alpine:3.20", "true"]);
        storage_quota_supported = quota_test
            .output()
            .context("probe Docker writable-layer quota support")?
            .status
            .success();
        if !storage_quota_supported {
            eprintln!(
                "Docker writable-layer quotas are unavailable; Scope will retain CPU, memory, and PID limits and clean workspaces after each attempt."
            );
        }
    }
    if !Path::new("/sys/fs/cgroup/cgroup.controllers").exists() {
        bail!("cgroups v2 is required");
    }
    command_success(
        Command::new("systemctl").args(["--user", "--version"]),
        "find systemd user service support",
    )?;
    Ok(DockerCapabilities {
        storage_quota_supported,
    })
}

pub(super) fn apply_container_limits(command: &mut Command, storage_quota_supported: bool) {
    command.args([
        "--memory",
        CONTAINER_MEMORY,
        "--memory-swap",
        CONTAINER_MEMORY,
        "--cpus",
        CONTAINER_CPUS,
        "--pids-limit",
        CONTAINER_PIDS,
    ]);
    if storage_quota_supported {
        command.args(["--storage-opt", &format!("size={CONTAINER_STORAGE}")]);
    }
}

pub(super) fn configure_job_container_creation(
    command: &mut Command,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    container_name: &str,
    container_image: &str,
    step_programs: &Path,
) {
    command.args(["create", "--name", container_name]);
    apply_container_limits(command, config.storage_quota_supported);
    command
        .args(["--label", &format!("scope.runner-id={}", config.runner_id)])
        .args(["--label", &format!("scope.attempt-id={}", claim.attempt_id)])
        .args([
            "--mount",
            &format!(
                "type=bind,source={},target=/scope-steps,readonly",
                step_programs.display()
            ),
        ])
        .args(["--entrypoint", "sh"])
        .args([
            container_image,
            "-c",
            "set -eu\n\
             if [ -d /scope-source ]; then\n\
               mkdir -p /workspace\n\
               cp -a /scope-source/. /workspace/\n\
               rm -rf /scope-source\n\
             fi\n\
             read -r phase step nonce < /scope-steps/current\n\
             [ \"$phase\" = prepare ]\n\
             : > /scope-step.log\n\
             cd /workspace\n\
             printf '%s\\n' \"$nonce\" > /scope-active-step.tmp\n\
             mv /scope-active-step.tmp /scope-active-step\n\
             while :; do\n\
               read -r next_phase next_step next_nonce < /scope-steps/current\n\
               if [ \"$next_phase\" = run ] && [ \"$next_step\" = \"$step\" ] && [ \"$next_nonce\" = \"$nonce\" ]; then\n\
                 break\n\
               fi\n\
               sleep 0.05\n\
             done\n\
             exec sh -e \"/scope-steps/step-$step.sh\" > /scope-step.log 2>&1",
        ]);
}

pub(super) fn container_started_at_unix(container_name: &str) -> anyhow::Result<u64> {
    let mut last_error = None;
    for _ in 0..50 {
        match container_timestamp_unix(
            container_name,
            "--format={{.State.StartedAt}}",
            "Docker job start time",
            true,
        ) {
            Ok(started_at) => return Ok(started_at),
            Err(error) => {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Err(last_error.expect("container start inspection records an error"))
}

pub(super) fn stop_container(container_name: &str) -> anyhow::Result<()> {
    let output = Command::new("docker")
        .args(["stop", "--time", "1", container_name])
        .output()
        .context("stop workflow step container")?;
    if container_is_stopped_or_missing(container_name)? {
        return Ok(());
    }
    if output.status.success() {
        bail!("workflow step container remained running after Docker stop");
    }
    bail!(
        "stop workflow step container: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn container_is_stopped_or_missing(container_name: &str) -> anyhow::Result<bool> {
    let output = Command::new("docker")
        .args([
            "container",
            "inspect",
            "--format={{.State.Running}}",
            container_name,
        ])
        .output()
        .context("confirm workflow step container stopped")?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim() == "false");
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.to_ascii_lowercase().contains("no such") {
        return Ok(true);
    }
    bail!("confirm workflow step container stopped: {}", stderr.trim())
}

fn container_timestamp_unix(
    container_name: &str,
    format: &str,
    label: &str,
    round_up: bool,
) -> anyhow::Result<u64> {
    let output = Command::new("docker")
        .args(["container", "inspect", format, container_name])
        .output()
        .with_context(|| format!("inspect {label}"))?;
    if !output.status.success() {
        bail!(
            "inspect {label}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let timestamp = String::from_utf8_lossy(&output.stdout);
    let timestamp = OffsetDateTime::parse(timestamp.trim(), &Rfc3339)
        .with_context(|| format!("parse {label}"))?;
    let seconds = u64::try_from(timestamp.unix_timestamp())
        .with_context(|| format!("{label} predates Unix time"))?;
    Ok(if round_up && timestamp.nanosecond() != 0 {
        seconds.saturating_add(1)
    } else {
        seconds
    })
}

pub(super) struct ContainerGuard {
    pub(super) name: String,
    cleanup_on_drop: bool,
}

impl ContainerGuard {
    pub(super) fn new(name: String) -> Self {
        Self {
            name,
            cleanup_on_drop: true,
        }
    }

    pub(super) fn preserve(&mut self) {
        self.cleanup_on_drop = false;
    }
}

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        if !self.cleanup_on_drop {
            return;
        }
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.name])
            .output();
    }
}
