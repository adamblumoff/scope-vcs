use super::*;
use std::{process::Command, time::Instant};

#[tokio::test]
async fn ingest_accepts_the_plaintext_limit_and_cleans_up_on_the_next_byte() {
    let fixture = Fixture::new(8, 64, 1);
    let exact = fixture
        .store
        .ingest(REPOSITORY_ID, &b"four"[..], 4)
        .await
        .unwrap();
    assert_eq!(exact.segment.plaintext_bytes, 4);
    fixture.store.delete_local(&exact).await.unwrap();
    fixture
        .store
        .delete_remote(&exact.object_key)
        .await
        .unwrap();

    let error = fixture
        .store
        .ingest(REPOSITORY_ID, &b"five!"[..], 4)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        GitStorageError::PlaintextLimitExceeded { max_bytes: 4 }
    ));
    assert!(fixture.backend.objects().is_empty());
    assert!(all_files(&fixture.local_root).await.is_empty());
}

#[tokio::test]
async fn zero_plaintext_limit_accepts_only_an_empty_stream() {
    let fixture = Fixture::new(8, 64, 1);
    let exact = fixture
        .store
        .ingest(REPOSITORY_ID, &b""[..], 0)
        .await
        .unwrap();
    assert_eq!(exact.segment.plaintext_bytes, 0);
    fixture.store.delete_local(&exact).await.unwrap();
    fixture
        .store
        .delete_remote(&exact.object_key)
        .await
        .unwrap();

    let error = fixture
        .store
        .ingest(REPOSITORY_ID, &b"x"[..], 0)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        GitStorageError::PlaintextLimitExceeded { max_bytes: 0 }
    ));
}

#[tokio::test]
async fn blocking_ingest_stops_after_the_first_byte_past_the_limit() {
    let fixture = Fixture::new(8, 64, 1);
    let consumed = Arc::new(AtomicUsize::new(0));
    let reader = TrackedReader {
        bytes: vec![b'x'; 64],
        offset: 0,
        consumed: Arc::clone(&consumed),
    };

    let error = fixture
        .store
        .ingest_blocking_reader(REPOSITORY_ID, reader, 4)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        GitStorageError::PlaintextLimitExceeded { max_bytes: 4 }
    ));
    assert_eq!(consumed.load(Ordering::SeqCst), 5);
    assert!(fixture.backend.objects().is_empty());
    assert!(all_files(&fixture.local_root).await.is_empty());
}

#[tokio::test]
async fn limit_cleanup_is_bounded_when_the_remote_backend_stalls() {
    let fixture = Fixture::new(8, 64, 1);
    fixture.backend.block_cleanup.store(true, Ordering::SeqCst);
    let reservation = fixture.store.reserve(REPOSITORY_ID).unwrap();
    let object_key = reservation.object_key.clone();
    let started = Instant::now();

    let error = fixture
        .store
        .ingest_reserved(REPOSITORY_ID, reservation, &b"five!"[..], 4)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        GitStorageError::PlaintextLimitExceeded { max_bytes: 4 }
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(fixture.backend.pending_for(&object_key), 1);
    assert!(all_files(&fixture.local_root).await.is_empty());

    fixture.backend.block_cleanup.store(false, Ordering::SeqCst);
    fixture.store.cleanup_remote(&object_key).await.unwrap();
}

#[tokio::test]
async fn process_timeout_cancels_a_stalled_multipart_ingest_without_detached_work() {
    let fixture = Fixture::new(4, 1, 1);
    fixture.backend.block_parts.store(true, Ordering::SeqCst);
    let reservation = fixture.store.reserve(REPOSITORY_ID).unwrap();
    let object_key = reservation.object_key.clone();
    let store = fixture.store.clone();
    let runtime = tokio::runtime::Handle::current();
    let started = Instant::now();

    let error = tokio::task::spawn_blocking(move || {
        let mut command = Command::new("sh");
        command.arg("-c").arg("printf 12345678; sleep 30");
        scope_git_process::run_with_stdout(
            &mut command,
            None,
            scope_git_process::ProcessLimits::new(Duration::from_millis(100)),
            "stalled Git segment ingest",
            move |stdout, cancellation| {
                runtime.block_on(store.ingest_reserved_blocking_reader_cancellable(
                    REPOSITORY_ID,
                    reservation,
                    stdout,
                    1024,
                    cancellation,
                ))
            },
        )
    })
    .await
    .unwrap()
    .unwrap_err();

    assert!(matches!(
        error,
        scope_git_process::StreamingProcessError::Process(
            scope_git_process::ProcessError::TimedOut { .. }
        )
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(fixture.backend.pending_for(&object_key), 0);
    assert!(fixture.backend.object(&object_key).is_none());
    assert!(all_files(&fixture.local_root).await.is_empty());
}

struct TrackedReader {
    bytes: Vec<u8>,
    offset: usize,
    consumed: Arc<AtomicUsize>,
}

impl Read for TrackedReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let remaining = &self.bytes[self.offset..];
        let read = remaining.len().min(output.len());
        output[..read].copy_from_slice(&remaining[..read]);
        self.offset += read;
        self.consumed.fetch_add(read, Ordering::SeqCst);
        Ok(read)
    }
}
