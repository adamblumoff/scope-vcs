use super::{AppendLogError, AppendLogOutcome, ExecutionSink};
use anyhow::{Context as _, anyhow};
use std::{
    io::{ErrorKind, Read},
    os::fd::AsRawFd,
    process::{ChildStderr, ChildStdout},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub(crate) const LOG_READ_BYTES: usize = 16 * 1024;
const LOG_QUEUE_CAPACITY: usize = 16;

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
enum OutputStream {
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
enum ReaderEvent {
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
        let (sender, receiver) = mpsc::sync_channel(LOG_QUEUE_CAPACITY);
        let stop_reading = Arc::new(AtomicBool::new(false));
        let locally_discarded = Arc::new(AtomicBool::new(false));
        let stdout_thread = spawn_reader(
            OutputStream::Stdout,
            stdout,
            sender.clone(),
            Arc::clone(&stop_reading),
            Arc::clone(&locally_discarded),
        );
        let stderr_thread = spawn_reader(
            OutputStream::Stderr,
            stderr,
            sender.clone(),
            Arc::clone(&stop_reading),
            Arc::clone(&locally_discarded),
        );
        drop(sender);

        let stop_uploading = Arc::new(AtomicBool::new(false));
        let stop_in_worker = Arc::clone(&stop_uploading);
        let (notice_sender, notices) = mpsc::channel();
        let upload_thread = thread::spawn(move || {
            let result = upload_output(
                receiver,
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
    sender: SyncSender<ReaderEvent>,
    stop_reading: Arc<AtomicBool>,
    locally_discarded: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || read_output(stream, reader, sender, stop_reading, locally_discarded))
}

fn read_output(
    stream: OutputStream,
    mut reader: impl Read,
    sender: SyncSender<ReaderEvent>,
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
                if !send_reader_event(
                    &sender,
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
                let _ = send_reader_event(
                    &sender,
                    ReaderEvent::Failed { stream, error },
                    &stop_reading,
                );
                break;
            }
        }
    }
}

fn send_reader_event(
    sender: &SyncSender<ReaderEvent>,
    mut event: ReaderEvent,
    stop_reading: &AtomicBool,
) -> bool {
    loop {
        if stop_reading.load(Ordering::Acquire) {
            return false;
        }
        match sender.try_send(event) {
            Ok(()) => return true,
            Err(TrySendError::Full(returned)) => {
                event = returned;
                thread::sleep(Duration::from_millis(10));
            }
            Err(TrySendError::Disconnected(_)) => return false,
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

#[allow(clippy::too_many_arguments)]
fn upload_output<S: ExecutionSink>(
    receiver: Receiver<ReaderEvent>,
    sink: &S,
    step: u32,
    mut next_sequence: u64,
    mut logs_truncated: bool,
    policy: UploadPolicy,
    stop_uploading: Arc<AtomicBool>,
    locally_discarded: Arc<AtomicBool>,
    notices: &mpsc::Sender<OutputNotice>,
) -> anyhow::Result<OutputSummary> {
    let mut pending_utf8 = [Vec::new(), Vec::new()];
    for event in receiver {
        match event {
            ReaderEvent::Chunk { .. }
                if logs_truncated || stop_uploading.load(Ordering::Acquire) =>
            {
                logs_truncated = true;
            }
            ReaderEvent::Chunk { stream, bytes } => {
                let (text, pending) = decode_utf8_chunk(
                    std::mem::take(&mut pending_utf8[stream.pending_index()]),
                    bytes,
                );
                pending_utf8[stream.pending_index()] = pending;
                if text.is_empty() {
                    continue;
                }
                let outcome =
                    append_with_retry(sink, step, next_sequence, &text, policy, &stop_uploading)?;
                match outcome {
                    Some(AppendLogOutcome::Accepted) => {
                        next_sequence = next_sequence
                            .checked_add(1)
                            .context("run log sequence overflow")?;
                    }
                    Some(AppendLogOutcome::Truncated) => {
                        logs_truncated = true;
                        let _ = notices.send(OutputNotice::Truncated);
                    }
                    None => logs_truncated = true,
                }
            }
            ReaderEvent::Failed { stream, error } => {
                return Err(error).with_context(|| format!("read step {}", stream.name()));
            }
        }
    }
    for pending in pending_utf8 {
        if pending.is_empty() {
            continue;
        }
        if logs_truncated || stop_uploading.load(Ordering::Acquire) {
            logs_truncated = true;
            continue;
        }
        let text = String::from_utf8_lossy(&pending);
        match append_with_retry(sink, step, next_sequence, &text, policy, &stop_uploading)? {
            Some(AppendLogOutcome::Accepted) => {
                next_sequence = next_sequence
                    .checked_add(1)
                    .context("run log sequence overflow")?;
            }
            Some(AppendLogOutcome::Truncated) | None => logs_truncated = true,
        }
    }
    logs_truncated |= locally_discarded.load(Ordering::Acquire);
    Ok(OutputSummary {
        next_sequence,
        logs_truncated,
    })
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
        let (sender, receiver) = mpsc::sync_channel(8);
        read_output_for_test(OutputStream::Stdout, Cursor::new(bytes), sender);
        let chunks = receiver
            .into_iter()
            .map(|event| match event {
                ReaderEvent::Chunk { bytes, .. } => bytes,
                ReaderEvent::Failed { error, .. } => panic!("unexpected read error: {error}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            chunks.iter().map(Vec::len).collect::<Vec<_>>(),
            [LOG_READ_BYTES, LOG_READ_BYTES, LOG_READ_BYTES, 7,]
        );
    }

    #[test]
    fn fixed_reader_reports_io_errors() {
        let (sender, receiver) = mpsc::sync_channel(4);
        read_output_for_test(
            OutputStream::Stderr,
            FailingReader {
                returned_bytes: false,
            },
            sender,
        );
        assert!(matches!(
            receiver.recv().unwrap(),
            ReaderEvent::Chunk { bytes, .. } if bytes == b"log"
        ));
        assert!(matches!(
            receiver.recv().unwrap(),
            ReaderEvent::Failed { stream: OutputStream::Stderr, error }
                if error.to_string() == "reader failed"
        ));
    }

    #[test]
    fn bounded_reader_waits_when_the_queue_is_full() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (done_sender, done_receiver) = mpsc::channel();
        let reader = thread::spawn(move || {
            read_output_for_test(
                OutputStream::Stdout,
                Cursor::new(vec![b'x'; LOG_READ_BYTES * 3]),
                sender,
            );
            done_sender.send(()).unwrap();
        });
        assert_eq!(
            done_receiver.recv_timeout(Duration::from_millis(50)),
            Err(RecvTimeoutError::Timeout)
        );
        drop(receiver);
        done_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        reader.join().unwrap();
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

    fn read_output_for_test(
        stream: OutputStream,
        reader: impl Read,
        sender: SyncSender<ReaderEvent>,
    ) {
        read_output(
            stream,
            reader,
            sender,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        );
    }
}
