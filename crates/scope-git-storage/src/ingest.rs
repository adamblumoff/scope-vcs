use super::{
    GitSegmentIngestTimings, GitSegmentReservation, GitSegmentStore, GitStorageError,
    MultipartError, MultipartStore, MultipartUpload, SegmentEncryptionKey, StagedGitSegment,
    UploadedPart, lifecycle::sync_directory, object_key, valid_segment_id,
};
use crate::envelope::EnvelopeWriter;
use bytes::Bytes;
use scope_domain::repository::git::GitSegmentRef;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, ReadBuf},
    sync::mpsc,
};

impl GitSegmentStore {
    pub async fn ingest<R>(
        &self,
        repository_id: &str,
        source: R,
    ) -> Result<StagedGitSegment, GitStorageError>
    where
        R: AsyncRead + Unpin + Send,
    {
        let reservation = self.reserve(repository_id)?;
        self.ingest_reserved(repository_id, reservation, source)
            .await
    }

    pub async fn ingest_blocking_reader<R>(
        &self,
        repository_id: &str,
        source: R,
    ) -> Result<StagedGitSegment, GitStorageError>
    where
        R: Read + Send + 'static,
    {
        let reservation = self.reserve(repository_id)?;
        self.ingest_reserved_blocking_reader(repository_id, reservation, source)
            .await
    }

    pub fn reserve(&self, repository_id: &str) -> Result<GitSegmentReservation, GitStorageError> {
        if repository_id.is_empty() {
            return Err(GitStorageError::InvalidConfiguration(
                "repository id is required".into(),
            ));
        }
        let segment_id = random_segment_id()?;
        Ok(GitSegmentReservation {
            object_key: object_key(repository_id, &segment_id),
            segment_id,
        })
    }

    pub async fn ingest_reserved<R>(
        &self,
        repository_id: &str,
        reservation: GitSegmentReservation,
        mut source: R,
    ) -> Result<StagedGitSegment, GitStorageError>
    where
        R: AsyncRead + Unpin + Send,
    {
        if repository_id.is_empty() || !valid_segment_id(&reservation.segment_id) {
            return Err(GitStorageError::InvalidConfiguration(
                "repository id and a generated segment id are required".into(),
            ));
        }
        let expected_key = object_key(repository_id, &reservation.segment_id);
        if reservation.object_key != expected_key {
            return Err(GitStorageError::InvalidConfiguration(
                "Git segment reservation does not belong to this repository".into(),
            ));
        }
        let started = Instant::now();
        let segment_id = reservation.segment_id;
        let local_dir = self.local_directory(repository_id);
        let object_key = reservation.object_key;
        let (local_tx, local_rx) = mpsc::channel(self.config.channel_capacity);
        let (remote_tx, remote_rx) = mpsc::channel(self.config.channel_capacity);
        let local_task = tokio::spawn(write_local(local_dir, segment_id.clone(), local_rx));
        let remote_task = tokio::spawn(upload_remote(
            RemoteIngestRequest {
                backend: Arc::clone(&self.backend),
                key: self.encryption_key.clone(),
                repository_id: repository_id.to_string(),
                segment_id: segment_id.clone(),
                object_key: object_key.clone(),
                frame_bytes: self.config.chunk_bytes,
                part_bytes: self.config.multipart_part_bytes,
            },
            remote_rx,
        ));

        let mut digest = Sha256::new();
        let mut plaintext_bytes = 0_u64;
        let mut fanout_blocked = Duration::ZERO;
        let mut input_error = None;
        let mut buffer = vec![0_u8; self.config.chunk_bytes];
        loop {
            match source.read(&mut buffer).await {
                Ok(0) => {
                    let blocked_at = Instant::now();
                    let (local_result, remote_result) = tokio::join!(
                        local_tx.send(StreamMessage::End),
                        remote_tx.send(StreamMessage::End),
                    );
                    fanout_blocked += blocked_at.elapsed();
                    if local_result.is_err() || remote_result.is_err() {
                        input_error = Some(GitStorageError::IncompleteIngest);
                    }
                    break;
                }
                Ok(read) => {
                    let chunk = Bytes::copy_from_slice(&buffer[..read]);
                    digest.update(&chunk);
                    plaintext_bytes =
                        plaintext_bytes.checked_add(read as u64).ok_or_else(|| {
                            GitStorageError::InvalidEnvelope("plaintext size overflow".into())
                        })?;
                    let blocked_at = Instant::now();
                    let (local_result, remote_result) = tokio::join!(
                        local_tx.send(StreamMessage::Chunk(chunk.clone())),
                        remote_tx.send(StreamMessage::Chunk(chunk)),
                    );
                    fanout_blocked += blocked_at.elapsed();
                    if local_result.is_err() || remote_result.is_err() {
                        input_error = Some(GitStorageError::IncompleteIngest);
                        break;
                    }
                }
                Err(error) => {
                    input_error = Some(GitStorageError::Input(error));
                    break;
                }
            }
        }
        drop(local_tx);
        drop(remote_tx);

        let (local, remote) = tokio::join!(local_task, remote_task);
        let local = local.map_err(|error| GitStorageError::Task(error.to_string()))?;
        let remote = remote.map_err(|error| GitStorageError::Task(error.to_string()))?;

        if let Some(error) = input_error {
            cleanup_ingest(&self.backend, &object_key, local.ok()).await;
            return Err(error);
        }
        let local = match local {
            Ok(outcome) => outcome,
            Err(error) => {
                cleanup_ingest(&self.backend, &object_key, None).await;
                return Err(error);
            }
        };
        let remote = match remote {
            Ok(outcome) => outcome,
            Err(error) => {
                cleanup_ingest(&self.backend, &object_key, Some(local)).await;
                return Err(error);
            }
        };
        let sha256 = hex::encode(digest.finalize());
        let segment = GitSegmentRef {
            segment_id,
            sha256,
            plaintext_bytes,
            encoding_version: crate::ENCODING_VERSION,
        };
        Ok(StagedGitSegment {
            segment,
            object_key,
            encrypted_bytes: remote.encrypted_bytes,
            key_id: self.encryption_key.key_id().to_string(),
            local_pack_path: local.path,
            timings: GitSegmentIngestTimings {
                total: started.elapsed(),
                local_write_and_fsync: local.elapsed,
                remote_multipart_upload: remote.elapsed,
                fanout_blocked,
                plaintext_bytes,
                encrypted_bytes: remote.encrypted_bytes,
                uploaded_parts: remote.uploaded_parts,
                chunk_bytes: self.config.chunk_bytes,
                channel_capacity: self.config.channel_capacity,
            },
        })
    }

    pub async fn ingest_reserved_blocking_reader<R>(
        &self,
        repository_id: &str,
        reservation: GitSegmentReservation,
        mut source: R,
    ) -> Result<StagedGitSegment, GitStorageError>
    where
        R: Read + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel(self.config.channel_capacity);
        let chunk_bytes = self.config.chunk_bytes;
        let bridge = tokio::task::spawn_blocking(move || {
            let mut buffer = vec![0_u8; chunk_bytes];
            loop {
                match source.read(&mut buffer) {
                    Ok(0) => return,
                    Ok(read) => {
                        if sender
                            .blocking_send(Ok(Bytes::copy_from_slice(&buffer[..read])))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = sender.blocking_send(Err(error));
                        return;
                    }
                }
            }
        });
        let result = self
            .ingest_reserved(
                repository_id,
                reservation,
                BlockingReaderStream::new(receiver),
            )
            .await;
        bridge
            .await
            .map_err(|error| GitStorageError::Task(error.to_string()))?;
        result
    }
}

#[derive(Debug)]
enum StreamMessage {
    Chunk(Bytes),
    End,
}

struct BlockingReaderStream {
    receiver: mpsc::Receiver<Result<Bytes, std::io::Error>>,
    current: Option<Bytes>,
}

impl BlockingReaderStream {
    fn new(receiver: mpsc::Receiver<Result<Bytes, std::io::Error>>) -> Self {
        Self {
            receiver,
            current: None,
        }
    }
}

impl AsyncRead for BlockingReaderStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        loop {
            if let Some(current) = &mut self.current {
                let take = output.remaining().min(current.len());
                output.put_slice(&current.split_to(take));
                if current.is_empty() {
                    self.current = None;
                }
                return Poll::Ready(Ok(()));
            }
            match self.receiver.poll_recv(context) {
                Poll::Ready(Some(Ok(bytes))) => self.current = Some(bytes),
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Err(error)),
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

struct LocalOutcome {
    path: PathBuf,
    elapsed: Duration,
}

struct RemoteOutcome {
    encrypted_bytes: u64,
    uploaded_parts: u32,
    elapsed: Duration,
}

async fn write_local(
    directory: PathBuf,
    segment_id: String,
    mut receiver: mpsc::Receiver<StreamMessage>,
) -> Result<LocalOutcome, GitStorageError> {
    let started = Instant::now();
    fs::create_dir_all(&directory)
        .await
        .map_err(GitStorageError::Local)?;
    let temp_path = directory.join(format!("{segment_id}.pack.tmp"));
    let final_path = directory.join(format!("{segment_id}.pack"));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)
        .await
        .map_err(GitStorageError::Local)?;
    let result = async {
        loop {
            match receiver.recv().await {
                Some(StreamMessage::Chunk(bytes)) => {
                    file.write_all(&bytes)
                        .await
                        .map_err(GitStorageError::Local)?;
                }
                Some(StreamMessage::End) => break,
                None => return Err(GitStorageError::IncompleteIngest),
            }
        }
        file.flush().await.map_err(GitStorageError::Local)?;
        file.sync_all().await.map_err(GitStorageError::Local)?;
        drop(file);
        if fs::try_exists(&final_path)
            .await
            .map_err(GitStorageError::Local)?
        {
            return Err(GitStorageError::Local(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "generated Git segment path already exists",
            )));
        }
        fs::rename(&temp_path, &final_path)
            .await
            .map_err(GitStorageError::Local)?;
        sync_directory(directory.clone()).await?;
        Ok(LocalOutcome {
            path: final_path,
            elapsed: started.elapsed(),
        })
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&temp_path).await;
    }
    result
}

struct RemoteIngestRequest {
    backend: Arc<dyn MultipartStore>,
    key: SegmentEncryptionKey,
    repository_id: String,
    segment_id: String,
    object_key: String,
    frame_bytes: usize,
    part_bytes: usize,
}

async fn upload_remote(
    request: RemoteIngestRequest,
    mut receiver: mpsc::Receiver<StreamMessage>,
) -> Result<RemoteOutcome, GitStorageError> {
    let RemoteIngestRequest {
        backend,
        key,
        repository_id,
        segment_id,
        object_key,
        frame_bytes,
        part_bytes,
    } = request;
    let started = Instant::now();
    let upload = backend.begin(&object_key).await?;
    let upload_for_abort = upload.clone();
    let result = async {
        let mut envelope = EnvelopeWriter::new(&key, &repository_id, &segment_id, frame_bytes)?;
        let mut parts = MultipartAccumulator::new(Arc::clone(&backend), upload.clone(), part_bytes);
        parts.push(envelope.header()).await?;
        loop {
            match receiver.recv().await {
                Some(StreamMessage::Chunk(bytes)) => {
                    let encrypted = envelope.encrypt_data(&bytes)?;
                    parts.push(&encrypted).await?;
                }
                Some(StreamMessage::End) => {
                    let final_frame = envelope.encrypt_final()?;
                    parts.push(&final_frame).await?;
                    break;
                }
                None => return Err(GitStorageError::IncompleteIngest),
            }
        }
        let (completed_parts, encrypted_bytes) = parts.finish().await?;
        let uploaded_parts = u32::try_from(completed_parts.len()).map_err(|_| {
            GitStorageError::InvalidEnvelope("multipart part count exceeds u32".into())
        })?;
        backend.complete(upload, completed_parts).await?;
        Ok(RemoteOutcome {
            encrypted_bytes,
            uploaded_parts,
            elapsed: started.elapsed(),
        })
    }
    .await;
    if result.is_err() {
        let _ = backend.abort(upload_for_abort).await;
    }
    result
}

struct MultipartAccumulator {
    backend: Arc<dyn MultipartStore>,
    upload: MultipartUpload,
    part_bytes: usize,
    buffer: Vec<u8>,
    parts: Vec<UploadedPart>,
    encrypted_bytes: u64,
}

impl MultipartAccumulator {
    fn new(backend: Arc<dyn MultipartStore>, upload: MultipartUpload, part_bytes: usize) -> Self {
        Self {
            backend,
            upload,
            part_bytes,
            buffer: Vec::with_capacity(part_bytes),
            parts: Vec::new(),
            encrypted_bytes: 0,
        }
    }

    async fn push(&mut self, mut bytes: &[u8]) -> Result<(), GitStorageError> {
        self.encrypted_bytes = self
            .encrypted_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| GitStorageError::InvalidEnvelope("encrypted size overflow".into()))?;
        while !bytes.is_empty() {
            let available = self.part_bytes - self.buffer.len();
            let take = available.min(bytes.len());
            self.buffer.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.buffer.len() == self.part_bytes {
                self.flush_part().await?;
            }
        }
        Ok(())
    }

    async fn finish(mut self) -> Result<(Vec<UploadedPart>, u64), GitStorageError> {
        if !self.buffer.is_empty() {
            self.flush_part().await?;
        }
        if self.parts.is_empty() {
            return Err(GitStorageError::InvalidEnvelope(
                "encrypted segment produced no multipart parts".into(),
            ));
        }
        Ok((self.parts, self.encrypted_bytes))
    }

    async fn flush_part(&mut self) -> Result<(), GitStorageError> {
        let part_number = i32::try_from(self.parts.len() + 1).map_err(|_| {
            GitStorageError::InvalidEnvelope("multipart part count exceeds i32".into())
        })?;
        let bytes = Bytes::from(std::mem::replace(
            &mut self.buffer,
            Vec::with_capacity(self.part_bytes),
        ));
        let part = self
            .backend
            .upload_part(&self.upload, part_number, bytes)
            .await?;
        if part.part_number != part_number {
            return Err(GitStorageError::Multipart(MultipartError::new(
                "multipart backend returned the wrong part number",
            )));
        }
        self.parts.push(part);
        Ok(())
    }
}

async fn cleanup_ingest(
    backend: &Arc<dyn MultipartStore>,
    object_key: &str,
    local: Option<LocalOutcome>,
) {
    if let Some(local) = local {
        let _ = fs::remove_file(local.path).await;
    }
    let _ = backend.abort_incomplete(object_key).await;
    let _ = backend.delete(object_key).await;
}

fn random_segment_id() -> Result<String, GitStorageError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        GitStorageError::InvalidConfiguration(format!("creating segment id: {error}"))
    })?;
    Ok(hex::encode(bytes))
}
