use super::{RunnerConfig, cache::CacheMount, command_success, resources::ResourceLimits};
use anyhow::{Context, bail};
use scope_api_contract::ClaimRunResponse;
use std::{env, path::Path, process::Command, thread, time::Duration};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct DockerCapabilities {
    pub(super) storage_quota_supported: bool,
}

pub(super) fn job_container_name(attempt_id: &str) -> String {
    format!("scope-{attempt_id}")
}

pub(super) fn doctor_local(
    run_container: bool,
    max_concurrent_jobs: scope_domain::runs::runner::RunnerMaxConcurrentJobs,
) -> anyhow::Result<(DockerCapabilities, ResourceLimits)> {
    if env::consts::OS != "linux" || env::consts::ARCH != "x86_64" {
        bail!("V5 runners require Linux on amd64");
    }
    command_success(Command::new("docker").args(["info"]), "connect to Docker")?;
    let limits = ResourceLimits::detect(max_concurrent_jobs)?;
    let capabilities = if run_container {
        let mut command = Command::new("docker");
        command.args(["run", "--rm"]);
        apply_container_limits(&mut command, &limits, DockerCapabilities::default());
        command.args(["alpine:3.20", "true"]);
        command_success(&mut command, "run bounded Docker test container")?;

        let mut quota_test = Command::new("docker");
        quota_test.args(["run", "--rm"]);
        apply_container_limits(
            &mut quota_test,
            &limits,
            DockerCapabilities {
                storage_quota_supported: true,
            },
        );
        quota_test.args(["alpine:3.20", "true"]);
        let storage_quota_supported = quota_test
            .output()
            .context("probe Docker writable-layer quota support")?
            .status
            .success();
        if !storage_quota_supported {
            eprintln!(
                "Docker writable-layer quotas are unavailable; Scope will use best-effort free-space admission and active monitoring instead."
            );
        }
        DockerCapabilities {
            storage_quota_supported,
        }
    } else {
        DockerCapabilities::default()
    };
    if !Path::new("/sys/fs/cgroup/cgroup.controllers").exists() {
        bail!("cgroups v2 is required");
    }
    command_success(
        Command::new("systemctl").args(["--user", "--version"]),
        "find systemd user service support",
    )?;
    Ok((capabilities, limits))
}

pub(super) fn probe_storage_quota_support(
    image: &str,
    limits: &ResourceLimits,
) -> anyhow::Result<DockerCapabilities> {
    let mut probe = Command::new("docker");
    configure_storage_quota_probe(&mut probe, image, limits);
    let output = probe
        .output()
        .context("probe Docker writable-layer quota support with workflow image")?;
    if !output.status.success() {
        eprintln!(
            "Docker writable-layer quotas are unavailable; Scope will use best-effort free-space admission and active monitoring instead."
        );
        return Ok(DockerCapabilities::default());
    }
    let container_id = std::str::from_utf8(&output.stdout)
        .context("quota probe returned a non-UTF-8 container ID")?
        .trim();
    if container_id.is_empty() || !container_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("quota probe returned an invalid Docker container ID");
    }
    let mut container = ContainerGuard::new(container_id.to_string());
    let mut cleanup = Command::new("docker");
    configure_storage_quota_cleanup(&mut cleanup, container_id);
    command_success(&mut cleanup, "remove Docker writable-layer quota probe")?;
    container.preserve();
    Ok(DockerCapabilities {
        storage_quota_supported: true,
    })
}

fn configure_storage_quota_probe(command: &mut Command, image: &str, limits: &ResourceLimits) {
    command.arg("create");
    apply_container_limits(
        command,
        limits,
        DockerCapabilities {
            storage_quota_supported: true,
        },
    );
    command.args(["--entrypoint", "sh", image, "-c", "true"]);
}

fn configure_storage_quota_cleanup(command: &mut Command, container_id: &str) {
    command.args(["container", "rm", "--force", "--volumes", container_id]);
}

pub(super) fn apply_container_limits(
    command: &mut Command,
    limits: &ResourceLimits,
    capabilities: DockerCapabilities,
) {
    limits.apply(command);
    if capabilities.storage_quota_supported {
        command.args(["--storage-opt", &format!("size={}", limits.storage_bytes)]);
    }
}

#[cfg(test)]
mod quota_tests {
    use super::*;

    #[test]
    fn quota_probe_uses_the_resolved_workflow_image_without_pulling() {
        let limits = ResourceLimits {
            memory_bytes: 1024,
            cpu_millis: 1000,
            pids: 128,
            storage_bytes: 2048,
        };
        let mut command = Command::new("docker");
        configure_storage_quota_probe(
            &mut command,
            "registry.example/workflow@sha256:abc",
            &limits,
        );
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(arguments.first().map(String::as_str), Some("create"));
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--storage-opt", "size=2048"])
        );
        assert!(arguments.windows(5).any(|arguments| {
            arguments
                == [
                    "--entrypoint",
                    "sh",
                    "registry.example/workflow@sha256:abc",
                    "-c",
                    "true",
                ]
        }));
        assert!(!arguments.iter().any(|argument| argument == "pull"));
        assert!(!arguments.iter().any(|argument| argument.contains("alpine")));

        let mut cleanup = Command::new("docker");
        configure_storage_quota_cleanup(&mut cleanup, "abc123");
        assert_eq!(
            cleanup
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["container", "rm", "--force", "--volumes", "abc123"]
        );
    }
}

pub(super) struct JobContainerSpec<'a> {
    pub(super) config: &'a RunnerConfig,
    pub(super) claim: &'a ClaimRunResponse,
    pub(super) name: &'a str,
    pub(super) image: &'a str,
    pub(super) step_programs: &'a Path,
    pub(super) limits: &'a ResourceLimits,
    pub(super) capabilities: DockerCapabilities,
    pub(super) caches: &'a [CacheMount],
}

pub(super) fn configure_job_container_creation(command: &mut Command, spec: JobContainerSpec<'_>) {
    command.args(["create", "--name", spec.name]);
    apply_container_limits(command, spec.limits, spec.capabilities);
    command
        .args([
            "--label",
            &format!("scope.runner-id={}", spec.config.runner_id),
        ])
        .args([
            "--label",
            &format!("scope.attempt-id={}", spec.claim.attempt_id),
        ])
        .args([
            "--mount",
            &format!(
                "type=bind,source={},target=/scope-steps,readonly",
                spec.step_programs.display()
            ),
        ])
        .args(["--entrypoint", "sh"]);
    for cache in spec.caches {
        command.args([
            "--mount",
            &format!(
                "type=volume,source={},target={}",
                cache.volume_name, cache.target
            ),
        ]);
    }
    command
        .args([
            spec.image,
            "-c",
            "set -eu\n\
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

pub(super) fn configure_source_copy(command: &mut Command, workspace: &Path, container: &str) {
    command
        .args(["cp"])
        .arg(format!("{}/.", workspace.display()))
        .arg(format!("{container}:/workspace"));
}

pub(super) fn require_root_image(image: &str) -> anyhow::Result<()> {
    // Image user metadata is only available after resolution. Root is a runner
    // contract because recovery must be able to clear every persistent cache entry.
    let output = Command::new("docker")
        .args(["image", "inspect", "--format={{.Config.User}}", image])
        .output()
        .context("inspect Docker image user")?;
    if !output.status.success() {
        bail!(
            "inspect Docker image user: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let user = String::from_utf8_lossy(&output.stdout);
    let user = user.trim();
    if image_user_is_root(user) {
        Ok(())
    } else {
        bail!(
            "workflow image runs as {user:?}; Scope V5 images must run as root because the runner populates /workspace and manages step state inside the container"
        )
    }
}

fn image_user_is_root(user: &str) -> bool {
    let account = user.split_once(':').map_or(user, |(account, _)| account);
    account.is_empty()
        || account == "root"
        || account
            .parse::<u32>()
            .is_ok_and(|numeric_user| numeric_user == 0)
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
            .args(["container", "rm", "--force", "--volumes", &self.name])
            .output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_image_users_accept_names_and_numeric_uid_zero_with_groups() {
        for root in ["", "root", "root:root", "root:123", "0", "0:root", "00:123"] {
            assert!(image_user_is_root(root), "{root:?}");
        }
        for non_root in ["runner", "runner:root", "1000", "1000:0", "rootless"] {
            assert!(!image_user_is_root(non_root), "{non_root:?}");
        }
    }

    #[test]
    fn source_copy_targets_workspace_contents_even_when_destination_exists() {
        let mut command = Command::new("docker");
        configure_source_copy(
            &mut command,
            Path::new("/runner/attempt/workspace"),
            "scope-a1",
        );
        let args = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            ["cp", "/runner/attempt/workspace/.", "scope-a1:/workspace"]
        );
    }
}
