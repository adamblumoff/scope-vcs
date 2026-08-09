use anyhow::{Context, bail};
use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};

pub(super) fn command_success(command: &mut Command, context: &str) -> anyhow::Result<()> {
    let output = command.output().with_context(|| context.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{context}: {}", stderr.trim());
    }
    Ok(())
}

pub(super) fn command_success_while(
    command: &mut Command,
    context: &str,
    should_continue: impl Fn() -> bool,
) -> anyhow::Result<()> {
    command_success_until(command, context, None, should_continue)
}

pub(super) fn command_success_while_for(
    command: &mut Command,
    context: &str,
    timeout: Duration,
    should_continue: impl Fn() -> bool,
) -> anyhow::Result<()> {
    command_success_until(command, context, Some(timeout), should_continue)
}

fn command_success_until(
    command: &mut Command,
    context: &str,
    timeout: Option<Duration>,
    should_continue: impl Fn() -> bool,
) -> anyhow::Result<()> {
    let started = timeout.map(|_| Instant::now());
    let mut child = command.spawn().with_context(|| context.to_string())?;
    loop {
        if !should_continue() {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{context}: runner storage crossed its emergency floor");
        }
        if let Some(status) = child.try_wait().with_context(|| context.to_string())? {
            if status.success() {
                return Ok(());
            }
            bail!("{context}: process exited with {status}");
        }
        if let (Some(timeout), Some(started)) = (timeout.as_ref(), started.as_ref())
            && started.elapsed() >= *timeout
        {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{context}: timed out after {} seconds",
                timeout.as_secs()
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn command_stdout(command: &mut Command, context: &str) -> anyhow::Result<String> {
    let output = command.output().with_context(|| context.to_string())?;
    if !output.status.success() {
        bail!(
            "{context}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(super) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub(super) struct RunnerWorkDir {
    pub(super) path: PathBuf,
    pub(super) cleanup_on_drop: bool,
}

impl RunnerWorkDir {
    pub(super) fn new(attempt_id: &str) -> anyhow::Result<Self> {
        let base = runner_work_root()?;
        fs::create_dir_all(&base).context("create runner work directory")?;
        let metadata = fs::symlink_metadata(&base)?;
        if !metadata.file_type().is_dir() {
            bail!("runner work path is not a directory");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&base, permissions)?;
        }
        let safe_id = attempt_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let path = base.join(safe_id);
        fs::create_dir(&path).context("create attempt work directory")?;
        Ok(Self {
            path,
            cleanup_on_drop: true,
        })
    }

    pub(super) fn preserve(&mut self) {
        self.cleanup_on_drop = false;
    }
}

pub(super) fn runner_work_root() -> anyhow::Result<PathBuf> {
    Ok(runner_state_home()?.join("scope/runner-work"))
}

fn runner_state_home() -> anyhow::Result<PathBuf> {
    env::var_os("XDG_STATE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|path| !path.is_empty())
                .map(|home| PathBuf::from(home).join(".local/state"))
        })
        .context("XDG_STATE_HOME or HOME is required")
}

impl Drop for RunnerWorkDir {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_command_stops_a_hung_process() {
        let error = command_success_while_for(
            Command::new("sh").args(["-c", "sleep 10"]),
            "bounded command",
            Duration::from_millis(50),
            || true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("timed out"));
    }
}
