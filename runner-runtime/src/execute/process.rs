use anyhow::{Context as _, anyhow};
use scope_domain::runs::workflow::definition::WorkflowJob;
use std::{
    os::unix::process::CommandExt as _,
    path::Path,
    process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(crate) struct StepProcess {
    child: Child,
    process_group: i32,
    status: Option<ExitStatus>,
    cleaned_up: bool,
}

impl StepProcess {
    pub(crate) fn spawn(
        command_text: &str,
        job: &WorkflowJob,
        workspace: &Path,
    ) -> anyhow::Result<Self> {
        let mut command = Command::new("sh");
        command
            .arg("-eu")
            .arg("-c")
            .arg(command_text)
            .current_dir(workspace)
            .envs(job.environment())
            .env_remove("SCOPE_BOOTSTRAP_TOKEN")
            .env_remove("SCOPE_ATTEMPT_DEADLINE_UNIX")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().context("spawn step process")?;
        let process_group = i32::try_from(child.id()).context("step process ID overflow")?;
        Ok(Self {
            child,
            process_group,
            status: None,
            cleaned_up: false,
        })
    }

    pub(crate) fn take_stdout(&mut self) -> anyhow::Result<ChildStdout> {
        self.child
            .stdout
            .take()
            .context("step process stdout was not captured")
    }

    pub(crate) fn take_stderr(&mut self) -> anyhow::Result<ChildStderr> {
        self.child
            .stderr
            .take()
            .context("step process stderr was not captured")
    }

    pub(crate) fn try_wait(&mut self) -> anyhow::Result<Option<ExitStatus>> {
        if self.status.is_none() {
            self.status = self.child.try_wait().context("inspect step process")?;
        }
        Ok(self.status)
    }

    pub(crate) fn signal_terminate(&self) -> anyhow::Result<()> {
        self.signal_group(libc::SIGTERM)
    }

    pub(crate) fn signal_kill(&self) -> anyhow::Result<()> {
        self.signal_group(libc::SIGKILL)
    }

    pub(crate) fn finish(mut self) -> anyhow::Result<ExitStatus> {
        self.signal_kill()?;
        self.reap()
    }

    pub(crate) fn terminate_and_wait(mut self, grace: Duration) -> anyhow::Result<ExitStatus> {
        self.signal_terminate()?;
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if self.try_wait()?.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.signal_kill()?;
        self.reap()
    }

    fn reap(&mut self) -> anyhow::Result<ExitStatus> {
        let status = match self.status {
            Some(status) => status,
            None => self.child.wait().context("wait for step process")?,
        };
        self.status = Some(status);
        self.cleaned_up = true;
        Ok(status)
    }

    fn signal_group(&self, signal: i32) -> anyhow::Result<()> {
        let result = unsafe { libc::kill(-self.process_group, signal) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        Err(anyhow!(error)).context("signal step process group")
    }
}

impl Drop for StepProcess {
    fn drop(&mut self) {
        if self.cleaned_up {
            return;
        }
        let _ = self.signal_group(libc::SIGKILL);
        if self.status.is_none() {
            let _ = self.child.wait();
        }
    }
}
