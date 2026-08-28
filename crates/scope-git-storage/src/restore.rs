use super::{
    ENCODING_VERSION, GitSegmentRestoreSource, GitSegmentRestoreTimings, GitSegmentStore,
    GitStorageError, MultipartError, object_key, valid_segment_id,
};
use crate::envelope::{DecryptedFrame, EnvelopeReader};
use scope_domain::repository::git::GitSegmentRef;
use sha2::{Digest, Sha256};
use std::time::Instant;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWrite, AsyncWriteExt},
};

impl GitSegmentStore {
    pub async fn restore_to<W>(
        &self,
        repository_id: &str,
        segment: &GitSegmentRef,
        mut output: W,
    ) -> Result<GitSegmentRestoreTimings, GitStorageError>
    where
        W: AsyncWrite + Unpin + Send,
    {
        validate_restore_identity(repository_id, segment)?;
        let started = Instant::now();
        let object_key = object_key(repository_id, &segment.segment_id);
        let mut source = self.backend.read(&object_key).await?;
        let mut envelope = EnvelopeReader::read_header(
            &mut source,
            &self.encryption_key,
            repository_id,
            &segment.segment_id,
        )
        .await?;
        let mut digest = Sha256::new();
        let mut plaintext_bytes = 0_u64;
        let mut frames = 0_u32;
        while let DecryptedFrame::Data(bytes) = envelope.next(&mut source).await? {
            output
                .write_all(&bytes)
                .await
                .map_err(GitStorageError::Output)?;
            digest.update(&bytes);
            plaintext_bytes = plaintext_bytes
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| {
                    GitStorageError::InvalidEnvelope("plaintext size overflow".into())
                })?;
            frames = frames
                .checked_add(1)
                .ok_or_else(|| GitStorageError::InvalidEnvelope("frame count overflow".into()))?;
        }
        let mut trailing = [0_u8; 1];
        if source
            .read(&mut trailing)
            .await
            .map_err(|error| GitStorageError::Multipart(MultipartError::new(error.to_string())))?
            != 0
        {
            return Err(GitStorageError::InvalidEnvelope(
                "data follows the final frame".into(),
            ));
        }
        output.flush().await.map_err(GitStorageError::Output)?;
        verify_plaintext(segment, plaintext_bytes, digest)?;
        Ok(GitSegmentRestoreTimings {
            total: started.elapsed(),
            plaintext_bytes,
            verified_frames: frames,
            source: GitSegmentRestoreSource::Remote,
        })
    }

    pub async fn restore_to_prefer_local<W>(
        &self,
        repository_id: &str,
        segment: &GitSegmentRef,
        output: W,
    ) -> Result<GitSegmentRestoreTimings, GitStorageError>
    where
        W: AsyncWrite + Unpin + Send,
    {
        validate_restore_identity(repository_id, segment)?;
        let started = Instant::now();
        let local_path = self.local_pack_path(repository_id, &segment.segment_id);
        match File::open(local_path).await {
            Ok(file) => {
                restore_plaintext_local(file, output, segment, self.config.chunk_bytes, started)
                    .await
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut timings = self.restore_to(repository_id, segment, output).await?;
                timings.total = started.elapsed();
                Ok(timings)
            }
            Err(error) => Err(GitStorageError::Local(error)),
        }
    }
}

fn validate_restore_identity(
    repository_id: &str,
    segment: &GitSegmentRef,
) -> Result<(), GitStorageError> {
    if segment.encoding_version != ENCODING_VERSION {
        return Err(GitStorageError::InvalidEnvelope(format!(
            "unsupported encoding version {}",
            segment.encoding_version
        )));
    }
    if repository_id.is_empty() || !valid_segment_id(&segment.segment_id) {
        return Err(GitStorageError::InvalidEnvelope(
            "repository id or segment id is invalid".into(),
        ));
    }
    Ok(())
}

async fn restore_plaintext_local<W>(
    mut source: File,
    mut output: W,
    segment: &GitSegmentRef,
    chunk_bytes: usize,
    started: Instant,
) -> Result<GitSegmentRestoreTimings, GitStorageError>
where
    W: AsyncWrite + Unpin + Send,
{
    let mut digest = Sha256::new();
    let mut plaintext_bytes = 0_u64;
    let mut buffer = vec![0_u8; chunk_bytes];
    loop {
        let read = source
            .read(&mut buffer)
            .await
            .map_err(GitStorageError::Local)?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .await
            .map_err(GitStorageError::Output)?;
        digest.update(&buffer[..read]);
        plaintext_bytes = plaintext_bytes
            .checked_add(read as u64)
            .ok_or_else(|| GitStorageError::InvalidEnvelope("plaintext size overflow".into()))?;
    }
    output.flush().await.map_err(GitStorageError::Output)?;
    verify_plaintext(segment, plaintext_bytes, digest)?;
    Ok(GitSegmentRestoreTimings {
        total: started.elapsed(),
        plaintext_bytes,
        verified_frames: 0,
        source: GitSegmentRestoreSource::Local,
    })
}

fn verify_plaintext(
    segment: &GitSegmentRef,
    plaintext_bytes: u64,
    digest: Sha256,
) -> Result<(), GitStorageError> {
    if plaintext_bytes != segment.plaintext_bytes {
        return Err(GitStorageError::SizeMismatch {
            expected: segment.plaintext_bytes,
            actual: plaintext_bytes,
        });
    }
    let actual = hex::encode(digest.finalize());
    if actual != segment.sha256 {
        return Err(GitStorageError::ChecksumMismatch {
            expected: segment.sha256.clone(),
            actual,
        });
    }
    Ok(())
}
