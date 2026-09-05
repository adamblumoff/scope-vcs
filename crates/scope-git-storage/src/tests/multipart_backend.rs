use super::*;

pub(super) struct TestMultipartStore {
    state: Mutex<TestState>,
    pub(super) fail_part: AtomicBool,
    pub(super) fail_complete: AtomicBool,
    pub(super) block_parts: AtomicBool,
    pub(super) part_delay_ms: AtomicUsize,
    pub(super) reorder_parts: AtomicBool,
    pub(super) active_parts: AtomicUsize,
    pub(super) peak_parts: AtomicUsize,
    pub(super) block_cleanup: AtomicBool,
    pub(super) part_started: Notify,
    pub(super) part_gate: Semaphore,
}

impl Default for TestMultipartStore {
    fn default() -> Self {
        Self {
            state: Mutex::new(TestState::default()),
            fail_part: AtomicBool::new(false),
            fail_complete: AtomicBool::new(false),
            block_parts: AtomicBool::new(false),
            part_delay_ms: AtomicUsize::new(0),
            reorder_parts: AtomicBool::new(false),
            active_parts: AtomicUsize::new(0),
            peak_parts: AtomicUsize::new(0),
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
    pub(super) fn object(&self, key: &str) -> Option<Bytes> {
        self.state.lock().unwrap().objects.get(key).cloned()
    }

    pub(super) fn objects(&self) -> HashMap<String, Bytes> {
        self.state.lock().unwrap().objects.clone()
    }

    pub(super) fn replace_object(&self, key: &str, bytes: Vec<u8>) {
        self.state
            .lock()
            .unwrap()
            .objects
            .insert(key.to_string(), Bytes::from(bytes));
    }

    pub(super) fn completed(&self) -> usize {
        self.state.lock().unwrap().completed
    }

    pub(super) fn aborted(&self) -> usize {
        self.state.lock().unwrap().aborted
    }

    pub(super) fn part_sizes(&self) -> Vec<usize> {
        self.state.lock().unwrap().last_part_sizes.clone()
    }

    pub(super) fn pending_for(&self, key: &str) -> usize {
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
        struct ActivePart<'a>(&'a AtomicUsize);
        impl Drop for ActivePart<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let active = self.active_parts.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak_parts.fetch_max(active, Ordering::SeqCst);
        let _active = ActivePart(&self.active_parts);
        self.part_started.notify_one();
        let delay = self.part_delay_ms.load(Ordering::SeqCst);
        if delay > 0 && (!self.reorder_parts.load(Ordering::SeqCst) || part_number % 2 == 1) {
            tokio::time::sleep(Duration::from_millis(delay as u64)).await;
        }
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

pub(super) struct MinimumS3PartStore;

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
