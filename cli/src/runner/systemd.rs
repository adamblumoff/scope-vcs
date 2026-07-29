use super::{command_success, scope_config_home};
use anyhow::{Context, bail};
use std::{env, fs, path::Path, process::Command};

const RUNNER_SERVICE_NAME: &str = "scope-runner.service";

pub(super) fn install_systemd_service(config_path: &Path) -> anyhow::Result<()> {
    let executable = env::current_exe().context("locate Scope binary")?;
    let unit_dir = scope_config_home()?.join("systemd/user");
    fs::create_dir_all(&unit_dir).context("create systemd user unit directory")?;
    let unit = systemd_unit(&executable, config_path)?;
    fs::write(unit_dir.join(RUNNER_SERVICE_NAME), unit).context("write systemd user unit")?;
    command_success(
        Command::new("systemctl").args(["--user", "daemon-reload"]),
        "reload systemd user units",
    )?;
    command_success(
        Command::new("systemctl").args(["--user", "enable", "--now", RUNNER_SERVICE_NAME]),
        "enable Scope runner service",
    )
}

fn systemd_unit(executable: &Path, config_path: &Path) -> anyhow::Result<String> {
    Ok(format!(
        "[Unit]\nDescription=Scope self-hosted runner\nAfter=network-online.target\n\n[Service]\nExecStart={} runner daemon --config {}\nRestart=always\nRestartSec=3\n\n[Install]\nWantedBy=default.target\n",
        systemd_quote_path(executable)?,
        systemd_quote_path(config_path)?
    ))
}

pub(super) fn systemd_quote_path(path: &Path) -> anyhow::Result<String> {
    let path = path
        .to_str()
        .context("Scope binary path must be valid UTF-8 for systemd")?;
    if path.contains(['\n', '\r']) {
        bail!("Scope binary path cannot contain a newline");
    }
    Ok(format!(
        "\"{}\"",
        path.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    ))
}

pub(super) fn print_linger_status() {
    let Some(user) = env::var("USER").ok().filter(|value| !value.is_empty()) else {
        return;
    };
    let output = Command::new("loginctl")
        .args(["show-user", &user, "--property=Linger", "--value"])
        .output();
    if output
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| String::from_utf8_lossy(&output.stdout).trim() != "yes")
    {
        eprintln!(
            "Runner starts now, but reboot persistence needs lingering. Run: sudo loginctl enable-linger {user}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_restarts_preserve_recovery_state() {
        let unit =
            systemd_unit(Path::new("/opt/scope"), Path::new("/etc/scope/runner.json")).unwrap();

        assert!(unit.contains("Restart=always"));
        assert!(!unit.contains("ExecStop"));
        assert!(!unit.contains("runner cleanup"));
    }
}
