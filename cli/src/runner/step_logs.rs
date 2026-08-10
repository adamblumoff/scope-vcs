use super::{
    LOG_CHUNK_BYTES, RunnerConfig, append_log_with_retry,
    recovery::{PendingLogChunk, stage_recovery_log_chunk, update_recovery_log_progress},
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_api_contract::ClaimRunResponse;
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    process::Command,
};

const CONTAINER_STEP_LOG: &str = "/scope-step.log";
const CONTAINER_STEP_LOG_TRUNCATED: &str = "/scope-step-truncated";
const RAW_LOG_CHUNK_BYTES: usize = LOG_CHUNK_BYTES / 4;

#[derive(Default)]
pub(super) struct StableLogDecoder {
    pending: Vec<u8>,
}

impl StableLogDecoder {
    pub(super) fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(valid) => {
                    text.push_str(valid);
                    self.pending.clear();
                    break;
                }
                Err(error) => {
                    let valid_bytes = error.valid_up_to();
                    if valid_bytes != 0 {
                        text.push_str(
                            std::str::from_utf8(&self.pending[..valid_bytes])
                                .expect("UTF-8 validator marked the prefix valid"),
                        );
                        self.pending.drain(..valid_bytes);
                    }
                    let Some(invalid_bytes) = error.error_len() else {
                        break;
                    };
                    for byte in self.pending.drain(..invalid_bytes) {
                        append_escaped_byte(&mut text, byte);
                    }
                }
            }
        }
        text
    }

    pub(super) fn finish(&mut self) -> String {
        let mut text = String::new();
        for byte in self.pending.drain(..) {
            append_escaped_byte(&mut text, byte);
        }
        text
    }
}

#[cfg(test)]
pub(super) fn stable_log_text(bytes: &[u8]) -> String {
    let mut decoder = StableLogDecoder::default();
    let mut text = decoder.push(bytes);
    text.push_str(&decoder.finish());
    text
}

fn append_escaped_byte(text: &mut String, byte: u8) {
    use std::fmt::Write as _;
    write!(text, "\\x{byte:02x}").expect("writing to a String cannot fail");
}

pub(super) fn copy_step_log(
    container_name: &str,
    destination: &Path,
) -> anyhow::Result<Option<u64>> {
    let output = Command::new("docker")
        .args(["cp", &format!("{container_name}:{CONTAINER_STEP_LOG}")])
        .arg(destination)
        .output()
        .context("copy workflow step log")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let normalized = stderr.to_ascii_lowercase();
        if normalized.contains("could not find") || normalized.contains("no such container") {
            return Ok(fs::metadata(destination)
                .ok()
                .map(|metadata| metadata.len()));
        }
        bail!("copy workflow step log: {}", stderr.trim());
    }
    Ok(Some(
        fs::metadata(destination)
            .context("inspect copied workflow step log")?
            .len(),
    ))
}

fn read_step_log_range(path: &Path, start: u64, length: usize) -> anyhow::Result<Vec<u8>> {
    let mut file = fs::File::open(path).context("open copied workflow step log")?;
    file.seek(SeekFrom::Start(start))
        .context("seek copied workflow step log")?;
    let mut bytes = Vec::with_capacity(length);
    file.take(length as u64)
        .read_to_end(&mut bytes)
        .context("read copied workflow step log range")?;
    Ok(bytes)
}

fn read_container_step_log_range(
    container_name: &str,
    snapshot: &Path,
    start: u64,
    length: usize,
) -> anyhow::Result<Vec<u8>> {
    let output = Command::new("docker")
        .args([
            "exec",
            container_name,
            "sh",
            "-c",
            &format!(
                "tail -c +{} {CONTAINER_STEP_LOG} | head -c {length}",
                start.saturating_add(1)
            ),
        ])
        .output()
        .context("read incremental workflow step log")?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let Some(log_len) = copy_step_log(container_name, snapshot)? else {
        return Ok(Vec::new());
    };
    if start > log_len {
        bail!("workflow step log became shorter during execution");
    }
    read_step_log_range(snapshot, start, length)
}

pub(super) fn step_log_was_truncated(
    container_name: &str,
    work_dir: &Path,
) -> anyhow::Result<bool> {
    let marker = work_dir.join("step-log-truncated");
    let _ = fs::remove_file(&marker);
    let output = Command::new("docker")
        .args([
            "cp",
            &format!("{container_name}:{CONTAINER_STEP_LOG_TRUNCATED}"),
        ])
        .arg(&marker)
        .output()
        .context("inspect workflow step log truncation marker")?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    if stderr.contains("could not find")
        || stderr.contains("no such file")
        || stderr.contains("no such container")
    {
        return Ok(false);
    }
    bail!(
        "inspect workflow step log truncation marker: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn drain_step_logs(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    work_dir: &Path,
    container_name: &str,
    step_index: u32,
    snapshot: &Path,
    next_log_sequence: &mut u64,
    step_log_bytes: &mut u64,
    logs_exhausted: &mut bool,
    pending_log_chunk: &mut Option<PendingLogChunk>,
    step_finished: bool,
) -> anyhow::Result<()> {
    if !upload_pending_step_log(
        client,
        config,
        claim,
        work_dir,
        next_log_sequence,
        step_log_bytes,
        logs_exhausted,
        pending_log_chunk,
    )? {
        return Ok(());
    }
    if *logs_exhausted && !step_finished {
        return Ok(());
    }
    let final_log_len = if step_finished {
        copy_step_log(container_name, snapshot)?
    } else {
        None
    };
    if step_finished && final_log_len.is_none() {
        return Ok(());
    }
    if final_log_len.is_some_and(|log_len| *step_log_bytes > log_len) {
        bail!("workflow step log became shorter during execution");
    }
    let mut cursor = *step_log_bytes;
    loop {
        let requested = final_log_len.map_or(RAW_LOG_CHUNK_BYTES, |log_len| {
            log_len
                .saturating_sub(cursor)
                .min(RAW_LOG_CHUNK_BYTES as u64) as usize
        });
        if requested == 0 {
            break;
        }
        let bytes = if step_finished {
            read_step_log_range(snapshot, cursor, requested)?
        } else {
            read_container_step_log_range(container_name, snapshot, cursor, requested)?
        };
        if bytes.is_empty() {
            break;
        }
        let final_chunk = final_log_len
            .is_some_and(|log_len| cursor.saturating_add(bytes.len() as u64) == log_len);
        let (text, consumed) = stable_log_prefix(&bytes, final_chunk);
        if consumed == 0 {
            break;
        }
        print!("{text}");
        let _ = std::io::stdout().flush();
        if *logs_exhausted {
            *step_log_bytes = step_log_bytes
                .checked_add(consumed as u64)
                .context("step log cursor overflow")?;
            cursor = *step_log_bytes;
            update_recovery_log_progress(
                work_dir,
                claim,
                step_index,
                *next_log_sequence,
                *step_log_bytes,
                true,
            )?;
            continue;
        }
        let pending = PendingLogChunk {
            step_index,
            sequence: *next_log_sequence,
            start_byte: *step_log_bytes,
            end_byte: step_log_bytes
                .checked_add(consumed as u64)
                .context("step log cursor overflow")?,
            text,
        };
        stage_recovery_log_chunk(work_dir, claim, pending.clone())?;
        *pending_log_chunk = Some(pending);
        if !upload_pending_step_log(
            client,
            config,
            claim,
            work_dir,
            next_log_sequence,
            step_log_bytes,
            logs_exhausted,
            pending_log_chunk,
        )? {
            return Ok(());
        }
        cursor = *step_log_bytes;
        if bytes.len() < RAW_LOG_CHUNK_BYTES {
            break;
        }
    }
    if step_finished && step_log_was_truncated(container_name, work_dir)? {
        *logs_exhausted = true;
        update_recovery_log_progress(
            work_dir,
            claim,
            step_index,
            *next_log_sequence,
            *step_log_bytes,
            true,
        )?;
    }
    Ok(())
}

fn stable_log_prefix(bytes: &[u8], finish: bool) -> (String, usize) {
    let mut decoder = StableLogDecoder::default();
    let mut text = decoder.push(bytes);
    let pending = decoder.pending.len();
    if finish {
        text.push_str(&decoder.finish());
        (text, bytes.len())
    } else {
        (text, bytes.len().saturating_sub(pending))
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn drain_recovered_step_logs(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    work_dir: &Path,
    container_name: &str,
    step_index: u32,
    next_log_sequence: u64,
    step_log_bytes: u64,
    logs_exhausted: bool,
    pending_log_chunk: Option<PendingLogChunk>,
) -> anyhow::Result<()> {
    let snapshot = work_dir.join(format!("step-{step_index}.log"));
    let mut next_log_sequence = next_log_sequence;
    let mut step_log_bytes = step_log_bytes;
    let mut logs_exhausted = logs_exhausted;
    let mut pending_log_chunk = pending_log_chunk;
    drain_step_logs(
        client,
        config,
        claim,
        work_dir,
        container_name,
        step_index,
        &snapshot,
        &mut next_log_sequence,
        &mut step_log_bytes,
        &mut logs_exhausted,
        &mut pending_log_chunk,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn upload_pending_step_log(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
    work_dir: &Path,
    next_log_sequence: &mut u64,
    step_log_bytes: &mut u64,
    logs_exhausted: &mut bool,
    pending_log_chunk: &mut Option<PendingLogChunk>,
) -> anyhow::Result<bool> {
    let Some(pending) = pending_log_chunk.clone() else {
        return Ok(true);
    };
    let accepted = match append_log_with_retry(
        client,
        config,
        claim,
        pending.step_index,
        pending.sequence,
        pending.text,
    ) {
        Ok(accepted) => accepted,
        Err(error) => {
            eprintln!(
                "\nScope log upload failed; execution will stop and recovery will retry: {error:#}"
            );
            return Err(error);
        }
    };
    if accepted {
        *next_log_sequence = pending
            .sequence
            .checked_add(1)
            .context("run log sequence overflow")?;
    } else {
        *logs_exhausted = true;
    }
    *step_log_bytes = pending.end_byte;
    update_recovery_log_progress(
        work_dir,
        claim,
        pending.step_index,
        *next_log_sequence,
        *step_log_bytes,
        *logs_exhausted,
    )?;
    *pending_log_chunk = None;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growing_log_defers_incomplete_utf8_until_the_next_snapshot() {
        let snowman = "☃".as_bytes();
        let (first, consumed) = stable_log_prefix(&snowman[..2], false);
        assert!(first.is_empty());
        assert_eq!(consumed, 0);

        let (complete, consumed) = stable_log_prefix(snowman, false);
        assert_eq!(complete, "☃");
        assert_eq!(consumed, snowman.len());

        let (trailing_invalid, consumed) = stable_log_prefix(&snowman[..2], true);
        assert_eq!(trailing_invalid, "\\xe2\\x98");
        assert_eq!(consumed, 2);
    }

    #[test]
    fn raw_log_chunk_bound_accounts_for_invalid_byte_expansion() {
        let bytes = vec![0xff; RAW_LOG_CHUNK_BYTES];
        let (text, consumed) = stable_log_prefix(&bytes, true);
        assert_eq!(consumed, RAW_LOG_CHUNK_BYTES);
        assert!(text.len() <= LOG_CHUNK_BYTES);
    }
}
