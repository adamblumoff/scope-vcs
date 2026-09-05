use super::{
    AppendLogError, AppendLogOutcome, ExecutionSink,
    spool::{LOG_SPOOL_BYTES, LogSpool},
};
use anyhow::{Context as _, anyhow};
use std::{
    io::{ErrorKind, Read},
    os::fd::AsRawFd,
    process::{ChildStderr, ChildStdout},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(crate) const LOG_READ_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UploadPolicy {
    pub(crate) attempts: usize,
    pub(crate) retry_delay: Duration,
}

impl Default for UploadPolicy {
    fn default() -> Self {
        Self {
            attempts: 3,
            retry_delay: Duration::from_millis(100),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum OutputStream {
    Stdout,
    Stderr,
}

impl OutputStream {
    fn name(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }

    fn pending_index(self) -> usize {
        match self {
            Self::Stdout => 0,
            Self::Stderr => 1,
        }
    }
}

#[derive(Debug)]
pub(super) enum ReaderEvent {
    Chunk {
        stream: OutputStream,
        bytes: Vec<u8>,
    },
    Failed {
        stream: OutputStream,
        error: std::io::Error,
    },
}

#[derive(Debug)]
pub(crate) struct OutputSummary {
    pub(crate) next_sequence: u64,
    pub(crate) logs_truncated: bool,
}

#[derive(Debug)]
pub(crate) enum OutputNotice {
    Truncated,
    Finished(OutputSummary),
    Failed(anyhow::Error),
}

pub(crate) struct OutputCapture {
    stop_uploading: Arc<AtomicBool>,
    stop_reading: Arc<AtomicBool>,
    notices: Receiver<OutputNotice>,
    threads: Vec<JoinHandle<()>>,
}

impl OutputCapture {
    pub(crate) fn start<S: ExecutionSink>(
        stdout: ChildStdout,
        stderr: ChildStderr,
        sink: Arc<S>,
        step: u32,
        next_sequence: u64,
        logs_truncated: bool,
        policy: UploadPolicy,
    ) -> anyhow::Result<Self> {
        set_nonblocking(&stdout).context("make step stdout nonblocking")?;
        set_nonblocking(&stderr).context("make step stderr nonblocking")?;
        let spool = Arc::new(LogSpool::new(LOG_SPOOL_BYTES, 2).context("create log spool")?);
        let stop_reading = Arc::new(AtomicBool::new(false));
        let locally_discarded = Arc::new(AtomicBool::new(false));
        let stdout_thread = spawn_reader(
            OutputStream::Stdout,
            stdout,
            Arc::clone(&spool),
            Arc::clone(&stop_reading),
            Arc::clone(&locally_discarded),
        );
        let stderr_thread = spawn_reader(
            OutputStream::Stderr,
            stderr,
            Arc::clone(&spool),
            Arc::clone(&stop_reading),
            Arc::clone(&locally_discarded),
        );

        let stop_uploading = Arc::new(AtomicBool::new(false));
        let stop_in_worker = Arc::clone(&stop_uploading);
        let (notice_sender, notices) = mpsc::channel();
        let upload_thread = thread::spawn(move || {
            let result = upload_output(
                spool,
                sink.as_ref(),
                step,
                next_sequence,
                logs_truncated,
                policy,
                stop_in_worker,
                locally_discarded,
                &notice_sender,
            );
            let notice = match result {
                Ok(summary) => OutputNotice::Finished(summary),
                Err(error) => OutputNotice::Failed(error),
            };
            let _ = notice_sender.send(notice);
        });

        Ok(Self {
            stop_uploading,
            stop_reading,
            notices,
            threads: vec![stdout_thread, stderr_thread, upload_thread],
        })
    }

    pub(crate) fn try_notice(&self) -> Option<OutputNotice> {
        self.notices.try_recv().ok()
    }

    pub(crate) fn finish_reading(&self) {
        self.stop_reading.store(true, Ordering::Release);
    }

    pub(crate) fn stop(&self) {
        self.stop_uploading.store(true, Ordering::Release);
        self.stop_reading.store(true, Ordering::Release);
    }

    pub(crate) fn wait(mut self) -> anyhow::Result<OutputSummary> {
        let notice = loop {
            match self
                .notices
                .recv()
                .context("runtime output worker stopped without a result")?
            {
                OutputNotice::Truncated => {}
                notice @ (OutputNotice::Finished(_) | OutputNotice::Failed(_)) => break notice,
            }
        };
        self.join_threads()?;
        match notice {
            OutputNotice::Finished(summary) => Ok(summary),
            OutputNotice::Failed(error) => Err(error),
            OutputNotice::Truncated => unreachable!("truncation is not a final output notice"),
        }
    }

    pub(crate) fn join(mut self) -> anyhow::Result<()> {
        self.join_threads()
    }

    fn join_threads(&mut self) -> anyhow::Result<()> {
        for thread in self.threads.drain(..) {
            thread
                .join()
                .map_err(|_| anyhow!("runtime output worker panicked"))?;
        }
        Ok(())
    }
}

fn spawn_reader(
    stream: OutputStream,
    reader: impl Read + Send + 'static,
    spool: Arc<LogSpool>,
    stop_reading: Arc<AtomicBool>,
    locally_discarded: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        read_output(
            stream,
            reader,
            Arc::clone(&spool),
            stop_reading,
            locally_discarded,
        );
        spool.close_reader();
    })
}

fn read_output(
    stream: OutputStream,
    mut reader: impl Read,
    spool: Arc<LogSpool>,
    stop_reading: Arc<AtomicBool>,
    locally_discarded: Arc<AtomicBool>,
) {
    let mut buffer = [0_u8; LOG_READ_BYTES];
    loop {
        if stop_reading.load(Ordering::Acquire) {
            if reader.read(&mut buffer).is_ok_and(|read| read > 0) {
                locally_discarded.store(true, Ordering::Release);
            }
            break;
        }
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if !spool.send(
                    ReaderEvent::Chunk {
                        stream,
                        bytes: buffer[..read].to_vec(),
                    },
                    &stop_reading,
                ) {
                    locally_discarded.store(true, Ordering::Release);
                    break;
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => {
                let _ = spool.send(ReaderEvent::Failed { stream, error }, &stop_reading);
                break;
            }
        }
    }
}

fn set_nonblocking(stream: &impl AsRawFd) -> anyhow::Result<()> {
    let descriptor = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(std::io::Error::last_os_error()).context("read pipe descriptor flags");
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error()).context("set pipe descriptor flags");
    }
    Ok(())
}

const LOG_FLUSH_INTERVAL: Duration = Duration::from_millis(50);
const LOG_UPLOAD_BYTES: usize = scope_domain::runs::log::MAX_RUN_LOG_CHUNK_BYTES;

#[allow(clippy::too_many_arguments)]
fn upload_output<S: ExecutionSink>(
    spool: Arc<LogSpool>,
    sink: &S,
    step: u32,
    next_sequence: u64,
    logs_truncated: bool,
    policy: UploadPolicy,
    stop_uploading: Arc<AtomicBool>,
    locally_discarded: Arc<AtomicBool>,
    notices: &mpsc::Sender<OutputNotice>,
) -> anyhow::Result<OutputSummary> {
    let mut upload = LogUpload {
        sink,
        step,
        next_sequence,
        logs_truncated,
        policy,
        stop_uploading: &stop_uploading,
        notices,
        pending: String::new(),
    };
    let mut pending_utf8 = [Vec::new(), Vec::new()];
    let mut flush_at = Instant::now() + LOG_FLUSH_INTERVAL;
    let mut discarding = logs_truncated;
    if discarding {
        spool.discard_output();
    }
    loop {
        match spool.recv_timeout(flush_at.saturating_duration_since(Instant::now())) {
            Ok(ReaderEvent::Chunk { stream, bytes }) => {
                let (text, pending) = decode_utf8_chunk(
                    std::mem::take(&mut pending_utf8[stream.pending_index()]),
                    bytes,
                );
                pending_utf8[stream.pending_index()] = pending;
                upload.pending.push_str(&text);
                upload.flush(false)?;
            }
            Ok(ReaderEvent::Failed { stream, error }) => {
                return Err(error).with_context(|| format!("read step {}", stream.name()));
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() >= flush_at {
            upload.flush(true)?;
            flush_at = Instant::now() + LOG_FLUSH_INTERVAL;
        }
        if upload.logs_truncated && !discarding {
            discarding = true;
            spool.discard_output();
        }
    }
    for pending in pending_utf8 {
        upload.pending.push_str(&String::from_utf8_lossy(&pending));
    }
    upload.flush(true)?;
    Ok(OutputSummary {
        next_sequence: upload.next_sequence,
        logs_truncated: upload.logs_truncated || locally_discarded.load(Ordering::Acquire),
    })
}

struct LogUpload<'a, S> {
    sink: &'a S,
    step: u32,
    next_sequence: u64,
    logs_truncated: bool,
    policy: UploadPolicy,
    stop_uploading: &'a AtomicBool,
    notices: &'a mpsc::Sender<OutputNotice>,
    pending: String,
}

impl<S: ExecutionSink> LogUpload<'_, S> {
    fn flush(&mut self, partial: bool) -> anyhow::Result<()> {
        while !self.pending.is_empty() {
            if self.logs_truncated || self.stop_uploading.load(Ordering::Acquire) {
                self.logs_truncated = true;
                self.pending.clear();
                break;
            }
            if !partial && self.pending.len() < LOG_UPLOAD_BYTES {
                break;
            }
            let mut end = self.pending.len().min(LOG_UPLOAD_BYTES);
            while !self.pending.is_char_boundary(end) {
                end -= 1;
            }
            match append_with_retry(
                self.sink,
                self.step,
                self.next_sequence,
                &self.pending[..end],
                self.policy,
                self.stop_uploading,
            )? {
                Some(AppendLogOutcome::Accepted) => {
                    self.next_sequence = self
                        .next_sequence
                        .checked_add(1)
                        .context("run log sequence overflow")?;
                }
                Some(AppendLogOutcome::Truncated) => {
                    self.logs_truncated = true;
                    let _ = self.notices.send(OutputNotice::Truncated);
                }
                None => self.logs_truncated = true,
            }
            self.pending.drain(..end);
        }
        Ok(())
    }
}

fn append_with_retry<S: ExecutionSink>(
    sink: &S,
    step: u32,
    sequence: u64,
    text: &str,
    policy: UploadPolicy,
    stop_uploading: &AtomicBool,
) -> anyhow::Result<Option<AppendLogOutcome>> {
    let attempts = policy.attempts.max(1);
    for attempt in 1..=attempts {
        if stop_uploading.load(Ordering::Acquire) {
            return Ok(None);
        }
        match sink.append_log(step, sequence, text) {
            Ok(outcome) => return Ok(Some(outcome)),
            Err(_) if stop_uploading.load(Ordering::Acquire) => {
                return Ok(None);
            }
            Err(AppendLogError::Retryable(_))
                if attempt < attempts && !stop_uploading.load(Ordering::Acquire) =>
            {
                thread::sleep(policy.retry_delay);
            }
            Err(error) => return Err(error.into_error()),
        }
    }
    unreachable!("append retry loop always returns")
}

fn decode_utf8_chunk(mut pending: Vec<u8>, bytes: Vec<u8>) -> (String, Vec<u8>) {
    pending.extend_from_slice(&bytes);
    let suffix = incomplete_utf8_suffix(&pending);
    let split_at = pending.len() - suffix;
    let remainder = pending.split_off(split_at);
    (String::from_utf8_lossy(&pending).into_owned(), remainder)
}

fn incomplete_utf8_suffix(bytes: &[u8]) -> usize {
    for length in 1..=bytes.len().min(3) {
        let suffix = &bytes[bytes.len() - length..];
        if let Err(error) = std::str::from_utf8(suffix)
            && error.valid_up_to() == 0
            && error.error_len().is_none()
        {
            return length;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Cursor, Error},
        sync::mpsc::RecvTimeoutError,
    };

    struct FailingReader {
        returned_bytes: bool,
    }

    impl Read for FailingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if !self.returned_bytes {
                self.returned_bytes = true;
                buffer[..3].copy_from_slice(b"log");
                Ok(3)
            } else {
                Err(Error::other("reader failed"))
            }
        }
    }

    #[test]
    fn fixed_reader_splits_newline_free_output() {
        let bytes = vec![b'x'; LOG_READ_BYTES * 3 + 7];
        let spool = Arc::new(LogSpool::new(LOG_SPOOL_BYTES, 1).unwrap());
        read_output_for_test(OutputStream::Stdout, Cursor::new(bytes), Arc::clone(&spool));
        let mut lengths = Vec::new();
        while let Ok(event) = spool.recv_timeout(Duration::ZERO) {
            match event {
                ReaderEvent::Chunk { bytes, .. } => lengths.push(bytes.len()),
                ReaderEvent::Failed { error, .. } => panic!("unexpected read error: {error}"),
            }
        }
        assert_eq!(lengths, [LOG_READ_BYTES, LOG_READ_BYTES, LOG_READ_BYTES, 7]);
    }

    #[test]
    fn fixed_reader_reports_io_errors() {
        let spool = Arc::new(LogSpool::new(LOG_SPOOL_BYTES, 1).unwrap());
        read_output_for_test(
            OutputStream::Stderr,
            FailingReader {
                returned_bytes: false,
            },
            Arc::clone(&spool),
        );
        assert!(matches!(spool.recv_timeout(Duration::ZERO).unwrap(),
            ReaderEvent::Chunk { bytes, .. } if bytes == b"log"));
        assert!(matches!(spool.recv_timeout(Duration::ZERO).unwrap(),
            ReaderEvent::Failed { stream: OutputStream::Stderr, error }
                if error.to_string() == "reader failed"));
    }

    #[test]
    fn bounded_reader_waits_for_spool_space_without_dropping_output() {
        let spool = Arc::new(LogSpool::new((LOG_READ_BYTES + 5) as u64, 1).unwrap());
        let (done_sender, done_receiver) = mpsc::channel();
        let reader_spool = Arc::clone(&spool);
        let reader = thread::spawn(move || {
            read_output_for_test(
                OutputStream::Stdout,
                Cursor::new(vec![b'x'; LOG_READ_BYTES * 3]),
                reader_spool,
            );
            done_sender.send(()).unwrap();
        });
        assert_eq!(
            done_receiver.recv_timeout(Duration::from_millis(50)),
            Err(RecvTimeoutError::Timeout)
        );
        let mut output = Vec::new();
        for _ in 0..3 {
            let ReaderEvent::Chunk { bytes, .. } =
                spool.recv_timeout(Duration::from_secs(1)).unwrap()
            else {
                panic!("expected log bytes")
            };
            output.extend(bytes);
        }
        done_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        reader.join().unwrap();
        assert_eq!(output, vec![b'x'; LOG_READ_BYTES * 3]);
    }

    #[test]
    fn lossy_utf8_expansion_stays_below_the_api_chunk_limit() {
        let invalid = vec![0xff; LOG_READ_BYTES];
        assert!(String::from_utf8_lossy(&invalid).len() <= 64 * 1024);
    }

    #[test]
    fn utf8_characters_survive_read_boundaries() {
        let (first, pending) = decode_utf8_chunk(Vec::new(), vec![b'a', 0xc3]);
        assert_eq!(first, "a");
        assert_eq!(pending, [0xc3]);

        let (second, pending) = decode_utf8_chunk(pending, vec![0xa9, b'b']);
        assert_eq!(second, "éb");
        assert!(pending.is_empty());
    }

    fn read_output_for_test(stream: OutputStream, reader: impl Read, spool: Arc<LogSpool>) {
        read_output(
            stream,
            reader,
            Arc::clone(&spool),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        );
        spool.close_reader();
    }
}
