use super::output::{OutputStream, ReaderEvent};
use std::{
    fs::File,
    io,
    os::unix::fs::FileExt as _,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::RecvTimeoutError,
    },
    time::{Duration, Instant},
};

pub(super) const LOG_SPOOL_BYTES: u64 = 64 * 1024 * 1024;
const HEADER_BYTES: usize = 5;

/// One bounded circular file shared by stdout and stderr. Readers wait only when the
/// spool is full; the uploader owns sequencing and releases space as it drains records.
pub(super) struct LogSpool {
    state: Mutex<State>,
    changed: Condvar,
    capacity: u64,
}

struct State {
    file: File,
    read_at: u64,
    write_at: u64,
    used: u64,
    readers: usize,
    failure: Option<ReaderEvent>,
    discarding: bool,
}

impl LogSpool {
    pub(super) fn new(capacity: u64, readers: usize) -> io::Result<Self> {
        let file = tempfile::tempfile()?;
        file.set_len(capacity)?;
        Ok(Self {
            state: Mutex::new(State {
                file,
                read_at: 0,
                write_at: 0,
                used: 0,
                readers,
                failure: None,
                discarding: false,
            }),
            changed: Condvar::new(),
            capacity,
        })
    }

    pub(super) fn send(&self, event: ReaderEvent, stop: &AtomicBool) -> bool {
        let mut state = self.state.lock().expect("log spool mutex poisoned");
        let ReaderEvent::Chunk { stream, bytes } = event else {
            state.failure = Some(event);
            self.changed.notify_all();
            return true;
        };
        if state.discarding {
            return true;
        }
        let record_bytes = HEADER_BYTES as u64 + bytes.len() as u64;
        assert!(
            record_bytes <= self.capacity,
            "log record exceeds spool capacity"
        );
        while self.capacity - state.used < record_bytes {
            if stop.load(Ordering::Acquire) || state.failure.is_some() {
                return false;
            }
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(10))
                .expect("log spool mutex poisoned")
                .0;
        }
        if state.discarding {
            return true;
        }
        if stop.load(Ordering::Acquire) || state.failure.is_some() {
            return false;
        }
        let mut header = [0; HEADER_BYTES];
        header[0] = match stream {
            OutputStream::Stdout => 0,
            OutputStream::Stderr => 1,
        };
        header[1..].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        let result = self
            .write(&state.file, state.write_at, &header)
            .and_then(|()| {
                self.write(
                    &state.file,
                    (state.write_at + HEADER_BYTES as u64) % self.capacity,
                    &bytes,
                )
            });
        if let Err(error) = result {
            state.failure = Some(ReaderEvent::Failed { stream, error });
            self.changed.notify_all();
            return false;
        }
        state.write_at = (state.write_at + record_bytes) % self.capacity;
        state.used += record_bytes;
        self.changed.notify_all();
        true
    }

    /// The API has confirmed truncation. Keep draining pipes without writing output
    /// that cannot be retained, including waking readers blocked on a full spool.
    pub(super) fn discard_output(&self) {
        let mut state = self.state.lock().expect("log spool mutex poisoned");
        state.discarding = true;
        state.used = 0;
        state.read_at = state.write_at;
        self.changed.notify_all();
    }

    pub(super) fn close_reader(&self) {
        let mut state = self.state.lock().expect("log spool mutex poisoned");
        state.readers -= 1;
        self.changed.notify_all();
    }

    pub(super) fn recv_timeout(&self, timeout: Duration) -> Result<ReaderEvent, RecvTimeoutError> {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().expect("log spool mutex poisoned");
        loop {
            if state.used > 0 {
                break;
            }
            if let Some(failure) = state.failure.take() {
                return Ok(failure);
            }
            if state.readers == 0 {
                return Err(RecvTimeoutError::Disconnected);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RecvTimeoutError::Timeout);
            }
            state = self
                .changed
                .wait_timeout(state, remaining)
                .expect("log spool mutex poisoned")
                .0;
        }
        let mut header = [0; HEADER_BYTES];
        let event = (|| {
            self.read(&state.file, state.read_at, &mut header)?;
            let length = u32::from_le_bytes(header[1..].try_into().expect("log length")) as usize;
            if length > super::output::LOG_READ_BYTES {
                return Err(io::Error::other("invalid log spool record length"));
            }
            let mut bytes = vec![0; length];
            self.read(
                &state.file,
                (state.read_at + HEADER_BYTES as u64) % self.capacity,
                &mut bytes,
            )?;
            let stream = match header[0] {
                0 => OutputStream::Stdout,
                1 => OutputStream::Stderr,
                _ => return Err(io::Error::other("invalid log spool stream")),
            };
            let record_bytes = HEADER_BYTES as u64 + length as u64;
            state.read_at = (state.read_at + record_bytes) % self.capacity;
            state.used -= record_bytes;
            Ok(ReaderEvent::Chunk { stream, bytes })
        })()
        .unwrap_or_else(|error| ReaderEvent::Failed {
            stream: OutputStream::Stdout,
            error,
        });
        self.changed.notify_all();
        Ok(event)
    }

    fn write(&self, file: &File, offset: u64, bytes: &[u8]) -> io::Result<()> {
        let split = bytes.len().min((self.capacity - offset) as usize);
        file.write_all_at(&bytes[..split], offset)?;
        file.write_all_at(&bytes[split..], 0)
    }

    fn read(&self, file: &File, offset: u64, bytes: &mut [u8]) -> io::Result<()> {
        let split = bytes.len().min((self.capacity - offset) as usize);
        file.read_exact_at(&mut bytes[..split], offset)?;
        file.read_exact_at(&mut bytes[split..], 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circular_file_wraps_records_without_growing_or_reordering_streams() {
        let spool = LogSpool::new(37, 1).unwrap();
        let stop = AtomicBool::new(false);
        let send = |value, stream| {
            assert!(spool.send(
                ReaderEvent::Chunk {
                    stream,
                    bytes: vec![value; 10]
                },
                &stop
            ))
        };
        send(b'a', OutputStream::Stdout);
        send(b'b', OutputStream::Stderr);
        assert!(
            matches!(spool.recv_timeout(Duration::ZERO).unwrap(), ReaderEvent::Chunk { bytes, .. } if bytes == vec![b'a'; 10])
        );
        send(b'c', OutputStream::Stdout);
        assert!(
            matches!(spool.recv_timeout(Duration::ZERO).unwrap(), ReaderEvent::Chunk { stream: OutputStream::Stderr, bytes } if bytes == vec![b'b'; 10])
        );
        assert!(
            matches!(spool.recv_timeout(Duration::ZERO).unwrap(), ReaderEvent::Chunk { stream: OutputStream::Stdout, bytes } if bytes == vec![b'c'; 10])
        );
        let state = spool.state.lock().unwrap();
        assert_eq!(state.used, 0);
        assert_eq!(state.file.metadata().unwrap().len(), 37);
    }

    #[test]
    fn confirmed_truncation_discards_buffered_and_future_output_without_disk_writes() {
        let spool = LogSpool::new(32, 1).unwrap();
        let stop = AtomicBool::new(false);
        assert!(spool.send(
            ReaderEvent::Chunk {
                stream: OutputStream::Stdout,
                bytes: vec![1; 10]
            },
            &stop
        ));
        spool.discard_output();
        // A read-only file proves future output never reaches the disk writer.
        spool.state.lock().unwrap().file = File::open("/dev/null").unwrap();
        assert!(spool.send(
            ReaderEvent::Chunk {
                stream: OutputStream::Stdout,
                bytes: vec![2; 10]
            },
            &stop
        ));
        spool.close_reader();
        assert!(matches!(
            spool.recv_timeout(Duration::ZERO),
            Err(RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn spool_disk_errors_are_reported_to_the_uploader() {
        let spool = LogSpool::new(32, 1).unwrap();
        spool.state.lock().unwrap().file = File::open("/dev/null").unwrap();
        assert!(!spool.send(
            ReaderEvent::Chunk {
                stream: OutputStream::Stdout,
                bytes: vec![1]
            },
            &AtomicBool::new(false)
        ));
        assert!(matches!(
            spool.recv_timeout(Duration::ZERO).unwrap(),
            ReaderEvent::Failed { .. }
        ));
    }
}
