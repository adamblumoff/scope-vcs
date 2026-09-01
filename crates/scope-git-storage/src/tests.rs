use super::*;
use async_trait::async_trait;
use bytes::Bytes;
use std::{
    collections::HashMap,
    io::{self, Read},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Notify, Semaphore},
};

const REPOSITORY_ID: &str = "repository-123";

mod ingest_control;

#[tokio::test]
async fn ingest_writes_both_destinations_and_restore_verifies_the_stream() {
    let fixture = Fixture::new(4, 13, 2);
    let input = b"abcdefghijklmnopqrstuvwxyz";

    let reservation = fixture.store.reserve(REPOSITORY_ID).unwrap();
    assert_eq!(
        reservation.object_key,
        object_key(REPOSITORY_ID, &reservation.segment_id)
    );
    let staged = fixture
        .store
        .ingest_reserved(REPOSITORY_ID, reservation, &input[..], u64::MAX)
        .await
        .unwrap();

    assert_eq!(staged.segment.encoding_version, ENCODING_VERSION);
    assert_eq!(staged.segment.plaintext_bytes, input.len() as u64);
    assert_eq!(
        tokio::fs::read(staged.local_pack_path()).await.unwrap(),
        input
    );
    assert!(staged.encrypted_bytes > input.len() as u64);
    assert_eq!(staged.timings.plaintext_bytes, input.len() as u64);
    assert!(staged.timings.fanout_blocked > Duration::ZERO);
    assert_eq!(
        staged.timings.uploaded_parts as usize,
        fixture.backend.part_sizes().len()
    );
    let part_sizes = fixture.backend.part_sizes();
    assert!(part_sizes.len() > 1);
    assert!(
        part_sizes[..part_sizes.len() - 1]
            .iter()
            .all(|size| *size == 13)
    );
    assert!(part_sizes.last().copied().unwrap() <= 13);
    assert_eq!(fixture.backend.completed(), 1);
    assert_eq!(fixture.backend.aborted(), 0);

    let (restored, timings) = restore_bytes(&fixture.store, &staged.segment)
        .await
        .unwrap();
    assert_eq!(restored, input);
    assert_eq!(timings.plaintext_bytes, input.len() as u64);
    assert_eq!(timings.verified_frames, 7);
    assert_eq!(timings.source, GitSegmentRestoreSource::Remote);

    fixture
        .store
        .delete_remote(&staged.object_key)
        .await
        .unwrap();
    assert!(fixture.backend.object(&staged.object_key).is_none());
    fixture.store.delete_local(&staged).await.unwrap();
    assert!(!staged.local_pack_path().exists());
}

#[tokio::test]
async fn blocking_reader_uses_the_reserved_identity() {
    let fixture = Fixture::new(5, 17, 1);
    let input = b"blocking child stdout data".to_vec();
    let reservation = fixture.store.reserve(REPOSITORY_ID).unwrap();
    let expected_id = reservation.segment_id.clone();

    let staged = fixture
        .store
        .ingest_reserved_blocking_reader(
            REPOSITORY_ID,
            reservation,
            std::io::Cursor::new(input.clone()),
            u64::MAX,
        )
        .await
        .unwrap();

    assert_eq!(staged.segment.segment_id, expected_id);
    assert_eq!(
        tokio::fs::read(staged.local_pack_path()).await.unwrap(),
        input
    );
}

#[tokio::test]
async fn preferred_restore_reads_and_verifies_the_stable_local_pack() {
    let fixture = Fixture::new(4, 13, 1);
    let input = b"warm local pack";
    let staged = fixture
        .store
        .ingest(REPOSITORY_ID, &input[..], u64::MAX)
        .await
        .unwrap();
    fixture.backend.delete(&staged.object_key).await.unwrap();

    let (restored, timings) = restore_preferred_bytes(&fixture.store, &staged.segment)
        .await
        .unwrap();

    assert_eq!(restored, input);
    assert_eq!(timings.source, GitSegmentRestoreSource::Local);
    assert_eq!(timings.plaintext_bytes, input.len() as u64);
    assert_eq!(timings.verified_frames, 0);
}

#[tokio::test]
async fn preferred_restore_uses_remote_only_when_local_pack_is_missing() {
    let fixture = Fixture::new(4, 13, 1);
    let input = b"cold remote pack";
    let staged = fixture
        .store
        .ingest(REPOSITORY_ID, &input[..], u64::MAX)
        .await
        .unwrap();
    tokio::fs::remove_file(staged.local_pack_path())
        .await
        .unwrap();

    let (restored, timings) = restore_preferred_bytes(&fixture.store, &staged.segment)
        .await
        .unwrap();

    assert_eq!(restored, input);
    assert_eq!(timings.source, GitSegmentRestoreSource::Remote);
    assert!(timings.verified_frames > 0);
}

#[tokio::test]
async fn preferred_restore_does_not_hide_local_corruption_with_remote_fallback() {
    let fixture = Fixture::new(4, 13, 1);
    let input = b"valid local pack";
    let staged = fixture
        .store
        .ingest(REPOSITORY_ID, &input[..], u64::MAX)
        .await
        .unwrap();
    tokio::fs::write(staged.local_pack_path(), b"broken local pac")
        .await
        .unwrap();

    let error = fixture
        .store
        .restore_to_prefer_local(REPOSITORY_ID, &staged.segment, tokio::io::sink())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        GitStorageError::ChecksumMismatch { .. } | GitStorageError::SizeMismatch { .. }
    ));
    assert!(fixture.backend.object(&staged.object_key).is_some());
}

#[tokio::test]
async fn preferred_restore_does_not_fall_back_on_local_io_errors() {
    let fixture = Fixture::new(4, 13, 1);
    let staged = fixture
        .store
        .ingest(REPOSITORY_ID, &b"valid remote copy"[..], u64::MAX)
        .await
        .unwrap();
    tokio::fs::remove_file(staged.local_pack_path())
        .await
        .unwrap();
    tokio::fs::create_dir(staged.local_pack_path())
        .await
        .unwrap();

    let error = fixture
        .store
        .restore_to_prefer_local(REPOSITORY_ID, &staged.segment, tokio::io::sink())
        .await
        .unwrap_err();

    assert!(matches!(error, GitStorageError::Local(_)));
}

#[tokio::test]
async fn filesystem_backend_completes_atomically_and_rejects_path_traversal() {
    let temp = tempfile::tempdir().unwrap();
    let backend = Arc::new(FileMultipartStore::new(temp.path().join("remote")).unwrap());
    let config = test_config(temp.path().join("local"), 4, 11, 1);
    let store = GitSegmentStore::new(backend.clone(), test_key(), config).unwrap();
    let input = b"filesystem multipart segment";

    let staged = store
        .ingest(REPOSITORY_ID, &input[..], u64::MAX)
        .await
        .unwrap();
    let (restored, _) = restore_bytes(&store, &staged.segment).await.unwrap();
    assert_eq!(restored, input);
    assert!(
        temp.path()
            .join("remote/objects")
            .join(&staged.object_key)
            .is_file()
    );
    assert!(
        all_files(&temp.path().join("remote/multipart"))
            .await
            .is_empty()
    );

    assert!(backend.begin("../outside").await.is_err());
    assert!(backend.read("git/segments/v2/../../outside").await.is_err());
    assert!(!temp.path().join("outside").exists());
}

#[tokio::test]
async fn failed_part_aborts_multipart_and_removes_local_output() {
    let fixture = Fixture::new(4, 8, 1);
    fixture.backend.fail_part.store(true, Ordering::SeqCst);

    let error = fixture
        .store
        .ingest(
            REPOSITORY_ID,
            &b"a pack that reaches the first part"[..],
            u64::MAX,
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        GitStorageError::IncompleteIngest | GitStorageError::Multipart(_)
    ));
    assert_eq!(fixture.backend.aborted(), 1);
    assert_eq!(fixture.backend.completed(), 0);
    assert!(all_files(&fixture.local_root).await.is_empty());
}

#[tokio::test]
async fn cleanup_remote_aborts_only_the_exact_key_then_deletes_its_object() {
    let fixture = Fixture::new(8, 64, 1);
    let staged = fixture
        .store
        .ingest(REPOSITORY_ID, &b"published object"[..], u64::MAX)
        .await
        .unwrap();
    fixture.backend.begin(&staged.object_key).await.unwrap();
    let neighbor_key = format!("{}-neighbor", staged.object_key);
    let neighbor = fixture.backend.begin(&neighbor_key).await.unwrap();

    fixture
        .store
        .cleanup_remote(&staged.object_key)
        .await
        .unwrap();

    assert!(fixture.backend.object(&staged.object_key).is_none());
    assert_eq!(fixture.backend.pending_for(&staged.object_key), 0);
    assert_eq!(fixture.backend.pending_for(&neighbor_key), 1);
    fixture.backend.abort(neighbor).await.unwrap();
}

#[tokio::test]
async fn cleanup_local_removes_exact_temp_and_completed_pack() {
    let fixture = Fixture::new(8, 64, 1);
    let reservation = fixture.store.reserve(REPOSITORY_ID).unwrap();
    let repository_hash = reservation.object_key.split('/').nth(3).unwrap();
    let directory = fixture.local_root.join(repository_hash);
    tokio::fs::create_dir_all(&directory).await.unwrap();
    let temp_path = directory.join(format!("{}.pack.tmp", reservation.segment_id));
    let pack_path = directory.join(format!("{}.pack", reservation.segment_id));
    tokio::fs::write(&temp_path, b"partial").await.unwrap();
    tokio::fs::write(&pack_path, b"complete").await.unwrap();

    fixture
        .store
        .cleanup_local(REPOSITORY_ID, &reservation.segment_id)
        .await
        .unwrap();

    assert!(!temp_path.exists());
    assert!(!pack_path.exists());
}

#[tokio::test]
async fn startup_cleanup_removes_only_the_local_staging_root() {
    let fixture = Fixture::new(8, 64, 1);
    let staged = fixture
        .store
        .ingest(REPOSITORY_ID, &b"published object"[..], u64::MAX)
        .await
        .unwrap();

    fixture.store.cleanup_all_local().await.unwrap();

    assert!(!fixture.local_root.exists());
    assert!(fixture.backend.object(&staged.object_key).is_some());
}

#[tokio::test]
async fn failed_complete_is_followed_by_abort() {
    let fixture = Fixture::new(8, 64, 1);
    fixture.backend.fail_complete.store(true, Ordering::SeqCst);

    let error = fixture
        .store
        .ingest(REPOSITORY_ID, &b"complete must fail"[..], u64::MAX)
        .await
        .unwrap_err();

    assert!(matches!(error, GitStorageError::Multipart(_)));
    assert_eq!(fixture.backend.aborted(), 1);
    assert!(fixture.backend.objects().is_empty());
    assert!(all_files(&fixture.local_root).await.is_empty());
}

#[tokio::test]
async fn local_failure_aborts_the_remote_upload() {
    let fixture_root = tempfile::tempdir().unwrap();
    let invalid_root = fixture_root.path().join("not-a-directory");
    tokio::fs::write(&invalid_root, b"file").await.unwrap();
    let backend = Arc::new(TestMultipartStore::default());
    let config = test_config(invalid_root, 4, 8, 1);
    let store = GitSegmentStore::new(backend.clone(), test_key(), config).unwrap();

    store
        .ingest(REPOSITORY_ID, &b"local write fails"[..], u64::MAX)
        .await
        .unwrap_err();

    assert_eq!(backend.aborted(), 1);
    assert!(backend.objects().is_empty());
}

#[tokio::test]
async fn bounded_channels_stop_a_blocking_reader_while_remote_is_slow() {
    let fixture = Fixture::new(4, 1, 1);
    fixture.backend.block_parts.store(true, Ordering::SeqCst);
    let reads = Arc::new(AtomicUsize::new(0));
    let reader = CountingReader {
        remaining: 20,
        reads: Arc::clone(&reads),
    };
    let store = fixture.store.clone();
    let task = tokio::spawn(async move {
        store
            .ingest_blocking_reader(REPOSITORY_ID, reader, u64::MAX)
            .await
    });

    tokio::time::timeout(
        Duration::from_secs(2),
        fixture.backend.part_started.notified(),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        reads.load(Ordering::SeqCst) <= 4,
        "bounded bridge read too far ahead"
    );

    fixture.backend.part_gate.add_permits(4096);
    let staged = task.await.unwrap().unwrap();
    assert_eq!(staged.segment.plaintext_bytes, 80);
}

#[tokio::test]
async fn restore_rejects_tampered_ciphertext() {
    let fixture = Fixture::new(4, 11, 1);
    let staged = fixture
        .store
        .ingest(REPOSITORY_ID, &b"tamper this ciphertext"[..], u64::MAX)
        .await
        .unwrap();
    let mut object = fixture.backend.object(&staged.object_key).unwrap().to_vec();
    let ranges = frame_ranges(&object);
    object[ranges[0].start + 10] ^= 0x40;
    fixture.backend.replace_object(&staged.object_key, object);

    let error = fixture
        .store
        .restore_to(REPOSITORY_ID, &staged.segment, tokio::io::sink())
        .await
        .unwrap_err();
    assert!(matches!(error, GitStorageError::InvalidEnvelope(_)));
}

#[tokio::test]
async fn restore_authenticates_the_envelope_header() {
    let fixture = Fixture::new(4, 11, 1);
    let staged = fixture
        .store
        .ingest(REPOSITORY_ID, &b"header authentication"[..], u64::MAX)
        .await
        .unwrap();
    let mut object = fixture.backend.object(&staged.object_key).unwrap().to_vec();
    object[14] ^= 0x20;
    fixture.backend.replace_object(&staged.object_key, object);

    let error = fixture
        .store
        .restore_to(REPOSITORY_ID, &staged.segment, tokio::io::sink())
        .await
        .unwrap_err();
    assert!(matches!(error, GitStorageError::InvalidEnvelope(_)));
}

#[tokio::test]
async fn restore_verifies_plaintext_size_and_whole_sha() {
    let fixture = Fixture::new(8, 32, 1);
    let staged = fixture
        .store
        .ingest(
            REPOSITORY_ID,
            &b"whole plaintext verification"[..],
            u64::MAX,
        )
        .await
        .unwrap();

    let mut wrong_size = staged.segment.clone();
    wrong_size.plaintext_bytes += 1;
    let size_error = fixture
        .store
        .restore_to(REPOSITORY_ID, &wrong_size, tokio::io::sink())
        .await
        .unwrap_err();
    assert!(matches!(size_error, GitStorageError::SizeMismatch { .. }));

    let mut wrong_sha = staged.segment.clone();
    wrong_sha.sha256 = "00".repeat(32);
    let checksum_error = fixture
        .store
        .restore_to(REPOSITORY_ID, &wrong_sha, tokio::io::sink())
        .await
        .unwrap_err();
    assert!(matches!(
        checksum_error,
        GitStorageError::ChecksumMismatch { .. }
    ));
}

#[tokio::test]
async fn restore_rejects_reordered_frames() {
    let fixture = Fixture::new(4, 9, 1);
    let staged = fixture
        .store
        .ingest(REPOSITORY_ID, &b"three-data-frames"[..], u64::MAX)
        .await
        .unwrap();
    let object = fixture.backend.object(&staged.object_key).unwrap().to_vec();
    let ranges = frame_ranges(&object);
    assert!(ranges.len() >= 3);
    let header_end = ranges[0].start;
    let mut reordered = object[..header_end].to_vec();
    reordered.extend_from_slice(&object[ranges[1].clone()]);
    reordered.extend_from_slice(&object[ranges[0].clone()]);
    for range in &ranges[2..] {
        reordered.extend_from_slice(&object[range.clone()]);
    }
    fixture
        .backend
        .replace_object(&staged.object_key, reordered);

    let error = fixture
        .store
        .restore_to(REPOSITORY_ID, &staged.segment, tokio::io::sink())
        .await
        .unwrap_err();
    assert!(matches!(error, GitStorageError::InvalidEnvelope(_)));
}

#[tokio::test]
async fn restore_rejects_truncation_and_bytes_after_final_frame() {
    let fixture = Fixture::new(4, 12, 1);
    let staged = fixture
        .store
        .ingest(REPOSITORY_ID, &b"authenticated final frame"[..], u64::MAX)
        .await
        .unwrap();
    let original = fixture.backend.object(&staged.object_key).unwrap().to_vec();

    fixture
        .backend
        .replace_object(&staged.object_key, original[..original.len() - 3].to_vec());
    let truncated = fixture
        .store
        .restore_to(REPOSITORY_ID, &staged.segment, tokio::io::sink())
        .await
        .unwrap_err();
    assert!(matches!(truncated, GitStorageError::InvalidEnvelope(_)));

    let mut extended = original;
    extended.push(0);
    fixture.backend.replace_object(&staged.object_key, extended);
    let trailing = fixture
        .store
        .restore_to(REPOSITORY_ID, &staged.segment, tokio::io::sink())
        .await
        .unwrap_err();
    assert!(matches!(trailing, GitStorageError::InvalidEnvelope(_)));
}

#[test]
fn s3_rejects_parts_smaller_than_five_mib() {
    let backend: Arc<dyn MultipartStore> = Arc::new(MinimumS3PartStore);
    let mut config = GitSegmentStoreConfig::new("/tmp/scope-git-storage-config-test");
    config.multipart_part_bytes = 5 * 1024 * 1024 - 1;

    let error = GitSegmentStore::new(backend, test_key(), config)
        .err()
        .expect("undersized S3 part must fail");
    assert!(matches!(error, GitStorageError::InvalidConfiguration(_)));
}

struct Fixture {
    _temp: tempfile::TempDir,
    local_root: std::path::PathBuf,
    backend: Arc<TestMultipartStore>,
    store: GitSegmentStore,
}

impl Fixture {
    fn new(chunk_bytes: usize, part_bytes: usize, channel_capacity: usize) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let local_root = temp.path().join("segments");
        let backend = Arc::new(TestMultipartStore::default());
        let config = test_config(
            local_root.clone(),
            chunk_bytes,
            part_bytes,
            channel_capacity,
        );
        let store = GitSegmentStore::new(backend.clone(), test_key(), config).unwrap();
        Self {
            _temp: temp,
            local_root,
            backend,
            store,
        }
    }
}

fn test_config(
    root: std::path::PathBuf,
    chunk_bytes: usize,
    part_bytes: usize,
    channel_capacity: usize,
) -> GitSegmentStoreConfig {
    GitSegmentStoreConfig {
        local_root: root,
        chunk_bytes,
        multipart_part_bytes: part_bytes,
        channel_capacity,
    }
}

fn test_key() -> SegmentEncryptionKey {
    SegmentEncryptionKey::new("key-1", [7_u8; 32]).unwrap()
}

async fn restore_bytes(
    store: &GitSegmentStore,
    segment: &GitSegmentRef,
) -> Result<(Vec<u8>, GitSegmentRestoreTimings), GitStorageError> {
    let (writer, mut reader) = tokio::io::duplex(segment.plaintext_bytes as usize + 1);
    let timings = store.restore_to(REPOSITORY_ID, segment, writer).await?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    Ok((bytes, timings))
}

async fn restore_preferred_bytes(
    store: &GitSegmentStore,
    segment: &GitSegmentRef,
) -> Result<(Vec<u8>, GitSegmentRestoreTimings), GitStorageError> {
    let (writer, mut reader) = tokio::io::duplex(segment.plaintext_bytes as usize + 1);
    let timings = store
        .restore_to_prefer_local(REPOSITORY_ID, segment, writer)
        .await?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    Ok((bytes, timings))
}

async fn all_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let Ok(mut repositories) = tokio::fs::read_dir(root).await else {
        return files;
    };
    while let Some(repository) = repositories.next_entry().await.unwrap() {
        let mut entries = tokio::fs::read_dir(repository.path()).await.unwrap();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            files.push(entry.path());
        }
    }
    files
}

fn frame_ranges(object: &[u8]) -> Vec<std::ops::Range<usize>> {
    let key_id_len = u16::from_be_bytes(object[12..14].try_into().unwrap()) as usize;
    let mut offset = 26 + key_id_len;
    let mut ranges = Vec::new();
    while offset < object.len() {
        let length =
            u32::from_be_bytes(object[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let end = offset + 9 + length + 16;
        ranges.push(offset..end);
        offset = end;
    }
    ranges
}

struct CountingReader {
    remaining: usize,
    reads: Arc<AtomicUsize>,
}

impl Read for CountingReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        self.remaining -= 1;
        self.reads.fetch_add(1, Ordering::SeqCst);
        let size = output.len().min(4);
        output[..size].fill(b'x');
        Ok(size)
    }
}

struct TestMultipartStore {
    state: Mutex<TestState>,
    fail_part: AtomicBool,
    fail_complete: AtomicBool,
    block_parts: AtomicBool,
    block_cleanup: AtomicBool,
    part_started: Notify,
    part_gate: Semaphore,
}

impl Default for TestMultipartStore {
    fn default() -> Self {
        Self {
            state: Mutex::new(TestState::default()),
            fail_part: AtomicBool::new(false),
            fail_complete: AtomicBool::new(false),
            block_parts: AtomicBool::new(false),
            block_cleanup: AtomicBool::new(false),
            part_started: Notify::new(),
            part_gate: Semaphore::new(0),
        }
    }
}

#[derive(Default)]
struct TestState {
    next_upload: usize,
    uploads: HashMap<String, TestUpload>,
    objects: HashMap<String, Bytes>,
    completed: usize,
    aborted: usize,
    last_part_sizes: Vec<usize>,
}

struct TestUpload {
    key: String,
    parts: HashMap<i32, Bytes>,
}

impl TestMultipartStore {
    fn object(&self, key: &str) -> Option<Bytes> {
        self.state.lock().unwrap().objects.get(key).cloned()
    }

    fn objects(&self) -> HashMap<String, Bytes> {
        self.state.lock().unwrap().objects.clone()
    }

    fn replace_object(&self, key: &str, bytes: Vec<u8>) {
        self.state
            .lock()
            .unwrap()
            .objects
            .insert(key.to_string(), Bytes::from(bytes));
    }

    fn completed(&self) -> usize {
        self.state.lock().unwrap().completed
    }

    fn aborted(&self) -> usize {
        self.state.lock().unwrap().aborted
    }

    fn part_sizes(&self) -> Vec<usize> {
        self.state.lock().unwrap().last_part_sizes.clone()
    }

    fn pending_for(&self, key: &str) -> usize {
        self.state
            .lock()
            .unwrap()
            .uploads
            .values()
            .filter(|upload| upload.key == key)
            .count()
    }
}

#[async_trait]
impl MultipartStore for TestMultipartStore {
    async fn begin(&self, key: &str) -> Result<MultipartUpload, MultipartError> {
        let mut state = self.state.lock().unwrap();
        state.next_upload += 1;
        let upload_id = state.next_upload.to_string();
        state.uploads.insert(
            upload_id.clone(),
            TestUpload {
                key: key.to_string(),
                parts: HashMap::new(),
            },
        );
        Ok(MultipartUpload {
            key: key.to_string(),
            upload_id,
        })
    }

    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: i32,
        bytes: Bytes,
    ) -> Result<UploadedPart, MultipartError> {
        self.part_started.notify_one();
        if self.block_parts.load(Ordering::SeqCst) {
            self.part_gate.acquire().await.unwrap().forget();
        }
        if self.fail_part.load(Ordering::SeqCst) {
            return Err(MultipartError::new("part failed"));
        }
        let mut state = self.state.lock().unwrap();
        let pending = state.uploads.get_mut(&upload.upload_id).unwrap();
        pending.parts.insert(part_number, bytes);
        Ok(UploadedPart {
            part_number,
            etag: format!("etag-{part_number}"),
        })
    }

    async fn complete(
        &self,
        upload: MultipartUpload,
        parts: Vec<UploadedPart>,
    ) -> Result<(), MultipartError> {
        if self.fail_complete.load(Ordering::SeqCst) {
            return Err(MultipartError::new("complete failed"));
        }
        let mut state = self.state.lock().unwrap();
        let mut pending = state.uploads.remove(&upload.upload_id).unwrap();
        assert_eq!(pending.key, upload.key);
        let mut object = Vec::new();
        let mut sizes = Vec::new();
        for part in parts {
            let bytes = pending.parts.remove(&part.part_number).unwrap();
            sizes.push(bytes.len());
            object.extend_from_slice(&bytes);
        }
        state.last_part_sizes = sizes;
        state.objects.insert(upload.key, Bytes::from(object));
        state.completed += 1;
        Ok(())
    }

    async fn abort(&self, upload: MultipartUpload) -> Result<(), MultipartError> {
        let mut state = self.state.lock().unwrap();
        state.uploads.remove(&upload.upload_id);
        state.aborted += 1;
        Ok(())
    }

    async fn abort_incomplete(&self, key: &str) -> Result<(), MultipartError> {
        if self.block_cleanup.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        let mut state = self.state.lock().unwrap();
        let aborted = state
            .uploads
            .values()
            .filter(|upload| upload.key == key)
            .count();
        state.uploads.retain(|_, upload| upload.key != key);
        state.aborted += aborted;
        Ok(())
    }

    async fn read(&self, key: &str) -> Result<RemoteReader, MultipartError> {
        let bytes = self
            .state
            .lock()
            .unwrap()
            .objects
            .get(key)
            .cloned()
            .ok_or_else(|| MultipartError::new("missing object"))?;
        let (mut writer, reader) = tokio::io::duplex(bytes.len().max(1));
        tokio::spawn(async move {
            writer.write_all(&bytes).await.unwrap();
        });
        Ok(Box::pin(reader))
    }

    async fn delete(&self, key: &str) -> Result<(), MultipartError> {
        self.state.lock().unwrap().objects.remove(key);
        Ok(())
    }
}

struct MinimumS3PartStore;

#[async_trait]
impl MultipartStore for MinimumS3PartStore {
    fn minimum_part_bytes(&self) -> usize {
        5 * 1024 * 1024
    }

    async fn begin(&self, _key: &str) -> Result<MultipartUpload, MultipartError> {
        unreachable!()
    }

    async fn upload_part(
        &self,
        _upload: &MultipartUpload,
        _part_number: i32,
        _bytes: Bytes,
    ) -> Result<UploadedPart, MultipartError> {
        unreachable!()
    }

    async fn complete(
        &self,
        _upload: MultipartUpload,
        _parts: Vec<UploadedPart>,
    ) -> Result<(), MultipartError> {
        unreachable!()
    }

    async fn abort(&self, _upload: MultipartUpload) -> Result<(), MultipartError> {
        unreachable!()
    }

    async fn abort_incomplete(&self, _key: &str) -> Result<(), MultipartError> {
        unreachable!()
    }

    async fn read(&self, _key: &str) -> Result<RemoteReader, MultipartError> {
        unreachable!()
    }

    async fn delete(&self, _key: &str) -> Result<(), MultipartError> {
        unreachable!()
    }
}
