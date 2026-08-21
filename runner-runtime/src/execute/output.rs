use super::{AppendLogError, AppendLogOutcome, ExecutionSink};
use anyhow::{Context as _, anyhow};
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub(crate) const LOG_READ_BYTES: usize = 16 * 1024;
const LOG_QUEUE_CAPACITY: usize = 16;

#[derive(Clone, Copy, Debug)]
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

#[derive(Debug)]
enum ReaderEvent {
    Chunk(Vec<u8>),
    Failed {
        stream: &'static str,
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
    notices: Receiver<OutputNotice>,
    threads: Vec<JoinHandle<()>>,
}

impl OutputCapture {
    pub(crate) fn start<S: ExecutionSink>(
        stdout: impl Read + Send + 'static,
        stderr: impl Read + Send + 'static,
        sink: Arc<S>,
        step: u32,
        next_sequence: u64,
        logs_truncated: bool,
        policy: UploadPolicy,
    ) -> Self {
        let (sender, receiver) = mpsc::sync_channel(LOG_QUEUE_CAPACITY);
        let stdout_thread = spawn_reader("stdout", stdout, sender.clone());
        let stderr_thread = spawn_reader("stderr", stderr, sender.clone());
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
                &notice_sender,
            );
            let notice = match result {
                Ok(summary) => OutputNotice::Finished(summary),
                Err(error) => OutputNotice::Failed(error),
            };
            let _ = notice_sender.send(notice);
        });

        Self {
            stop_uploading,
            notices,
            threads: vec![stdout_thread, stderr_thread, upload_thread],
        }
    }

    pub(crate) fn try_notice(&self) -> Option<OutputNotice> {
        self.notices.try_recv().ok()
    }

    pub(crate) fn stop_uploading(&self) {
        self.stop_uploading.store(true, Ordering::Release);
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
    stream: &'static str,
    reader: impl Read + Send + 'static,
    sender: SyncSender<ReaderEvent>,
) -> JoinHandle<()> {
    thread::spawn(move || read_output(stream, reader, sender))
}

fn read_output(stream: &'static str, mut reader: impl Read, sender: SyncSender<ReaderEvent>) {
    let mut buffer = [0_u8; LOG_READ_BYTES];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if sender
                    .send(ReaderEvent::Chunk(buffer[..read].to_vec()))
                    .is_err()
                {
                    break;
                }
            }
            Err(error) => {
                let _ = sender.send(ReaderEvent::Failed { stream, error });
                break;
            }
        }
    }
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
    notices: &mpsc::Sender<OutputNotice>,
) -> anyhow::Result<OutputSummary> {
    for event in receiver {
        match event {
            ReaderEvent::Chunk(_) if logs_truncated || stop_uploading.load(Ordering::Acquire) => {}
            ReaderEvent::Chunk(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
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
                    None => {}
                }
            }
            ReaderEvent::Failed { stream, error } => {
                return Err(error).with_context(|| format!("read step {stream}"));
            }
        }
    }
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
        read_output("stdout", Cursor::new(bytes), sender);
        let chunks = receiver
            .into_iter()
            .map(|event| match event {
                ReaderEvent::Chunk(bytes) => bytes,
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
        read_output(
            "stderr",
            FailingReader {
                returned_bytes: false,
            },
            sender,
        );
        assert!(matches!(receiver.recv().unwrap(), ReaderEvent::Chunk(bytes) if bytes == b"log"));
        assert!(matches!(
            receiver.recv().unwrap(),
            ReaderEvent::Failed { stream: "stderr", error } if error.to_string() == "reader failed"
        ));
    }

    #[test]
    fn bounded_reader_waits_when_the_queue_is_full() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (done_sender, done_receiver) = mpsc::channel();
        let reader = thread::spawn(move || {
            read_output(
                "stdout",
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
}
