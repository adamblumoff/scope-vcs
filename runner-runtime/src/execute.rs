use crate::api::RuntimeClient;
use anyhow::Context as _;
use scope_domain::runs::workflow::WorkflowJob;
use std::{
    io::{BufRead, BufReader},
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const LOG_CHUNK_BYTES: usize = 48 * 1024;

pub enum ExecutionOutcome {
    Succeeded,
    Terminal,
}

pub fn run_steps(
    client: &RuntimeClient,
    job: &WorkflowJob,
    workspace: &Path,
) -> anyhow::Result<ExecutionOutcome> {
    let deadline = Instant::now() + Duration::from_secs(job.timeout_seconds());
    let mut sequence = 1_u64;
    for (index, step) in job.steps().iter().enumerate() {
        let index = u32::try_from(index).context("step index overflow")?;
        if client.start_step(index)?.cancellation_requested {
            client.complete_canceled()?;
            return Ok(ExecutionOutcome::Terminal);
        }
        let mut command = Command::new("sh");
        command
            .arg("-eu")
            .arg("-c")
            .arg(step.run())
            .current_dir(workspace)
            .envs(job.environment())
            .env_remove("SCOPE_BOOTSTRAP_TOKEN")
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
        let mut child = command
            .spawn()
            .with_context(|| format!("start step {}", step.name()))?;
        let pid = child.id() as i32;
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        if let Some(stream) = child.stdout.take() {
            let tx = tx.clone();
            thread::spawn(move || {
                let mut reader = BufReader::new(stream);
                loop {
                    let mut bytes = Vec::new();
                    match reader.read_until(b'\n', &mut bytes) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let _ = tx.send(bytes);
                        }
                    }
                }
            });
        }
        if let Some(stream) = child.stderr.take() {
            let tx = tx.clone();
            thread::spawn(move || {
                let mut reader = BufReader::new(stream);
                loop {
                    let mut bytes = Vec::new();
                    match reader.read_until(b'\n', &mut bytes) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let _ = tx.send(bytes);
                        }
                    }
                }
            });
        }
        drop(tx);
        let mut next_heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
        loop {
            while let Ok(bytes) = rx.try_recv() {
                for chunk in bytes.chunks(LOG_CHUNK_BYTES) {
                    client.append_log(
                        index,
                        sequence,
                        String::from_utf8_lossy(chunk).into_owned(),
                    )?;
                    sequence += 1;
                }
            }
            if let Some(status) = child.try_wait().context("inspect step process")? {
                for bytes in rx.try_iter() {
                    for chunk in bytes.chunks(LOG_CHUNK_BYTES) {
                        client.append_log(
                            index,
                            sequence,
                            String::from_utf8_lossy(chunk).into_owned(),
                        )?;
                        sequence += 1;
                    }
                }
                let code = status.code().unwrap_or(128);
                client.complete_step(index, code)?;
                if code != 0 {
                    return Ok(ExecutionOutcome::Terminal);
                }
                break;
            }
            if Instant::now() >= deadline {
                kill_group(pid);
                let _ = child.wait();
                client.complete_timeout()?;
                return Ok(ExecutionOutcome::Terminal);
            }
            if Instant::now() >= next_heartbeat {
                if client.heartbeat()?.cancellation_requested {
                    kill_group(pid);
                    let _ = child.wait();
                    client.complete_canceled()?;
                    return Ok(ExecutionOutcome::Terminal);
                }
                next_heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(ExecutionOutcome::Succeeded)
}

fn kill_group(pid: i32) {
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    thread::sleep(Duration::from_secs(2));
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
}
