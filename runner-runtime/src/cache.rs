use crate::api::RuntimeClient;
use anyhow::{Context as _, bail};
use scope_api_contract::{
    AttemptCacheFinalizationReport, AttemptCachePreparationReport, CommitCacheUploadRequest,
    ReportAttemptCacheFinalizationsRequest, ReportAttemptCachePreparationsRequest, RunJobResponse,
};
use scope_domain::runs::{
    cache::{
        CacheColdReason, CacheFinalState, CacheIdentity, CacheNamespace, CachePlatform,
        CachePreparation,
    },
    run::PinnedContainerImage,
};
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

const MAX_CACHE_BYTES: u64 = 10 * 1024 * 1024 * 1024;

pub struct PreparedCache {
    digest: String,
    path: PathBuf,
}

pub fn prepare_caches(
    client: &RuntimeClient,
    job: &RunJobResponse,
) -> anyhow::Result<Vec<PreparedCache>> {
    let image = PinnedContainerImage::parse(job.pinned_container_image.clone())?;
    let workflow_path =
        scope_domain::runs::workflow::WorkflowPath::parse(job.workflow_path.clone())?;
    let job_key = scope_domain::runs::workflow::WorkflowJobId::parse(job.job_key.clone())?;
    let namespace = CacheNamespace::workflow(&workflow_path, &job_key);
    let mut prepared = Vec::new();
    let mut reports = Vec::new();
    for cache in job.definition.caches() {
        let started = Instant::now();
        let digest = CacheIdentity::new(
            &job.repository_id,
            namespace.clone(),
            cache.clone(),
            &image,
            CachePlatform::LinuxAmd64,
        )?
        .digest();
        let path = PathBuf::from(cache.mount_path());
        fs::create_dir_all(&path)
            .with_context(|| format!("create cache path {}", path.display()))?;
        let session = client.cache_download_session(&digest)?;
        let preparation = match (
            session.download_url,
            session.checksum_sha256,
            session.size_bytes,
        ) {
            (Some(url), Some(checksum), Some(size)) => {
                let temp_dir = tempfile::tempdir().context("create cache download directory")?;
                let archive = temp_dir.path().join("cache.tar.zst");
                client.download_cache(&url, &archive, size, &checksum)?;
                extract_archive(&archive, &path)?;
                CachePreparation::Warm
            }
            (None, None, None) => CachePreparation::Cold {
                reason: CacheColdReason::MetadataMissing,
            },
            _ => bail!("cache download session is inconsistent"),
        };
        reports.push(AttemptCachePreparationReport {
            cache_name: cache.as_str().to_string(),
            identity_digest: digest.clone(),
            preparation,
            prepare_ms: elapsed_ms(started),
        });
        prepared.push(PreparedCache { digest, path });
    }
    client.report_cache_preparations(&ReportAttemptCachePreparationsRequest { caches: reports })?;
    Ok(prepared)
}

pub fn save_caches(client: &RuntimeClient, caches: &[PreparedCache]) -> anyhow::Result<()> {
    let mut reports = Vec::new();
    for cache in caches {
        let started = Instant::now();
        let temp = tempfile::NamedTempFile::new().context("create cache upload file")?;
        create_archive(&cache.path, temp.path())?;
        let (size_bytes, checksum_sha256) = file_identity(temp.path())?;
        let session = client.cache_upload_session(&cache.digest)?;
        client.upload_cache(&session.upload_url, temp.path())?;
        client.commit_cache(
            &cache.digest,
            &CommitCacheUploadRequest {
                generation: session.generation,
                checksum_sha256,
                size_bytes,
            },
        )?;
        reports.push(AttemptCacheFinalizationReport {
            identity_digest: cache.digest.clone(),
            final_state: CacheFinalState::Ready,
            finalize_ms: elapsed_ms(started),
        });
    }
    client.report_cache_finalizations(&ReportAttemptCacheFinalizationsRequest { caches: reports })
}

fn extract_archive(archive: &Path, destination: &Path) -> anyhow::Result<()> {
    let file = fs::File::open(archive)?;
    let decoder = zstd::Decoder::new(file).context("open compressed cache")?;
    tar::Archive::new(decoder)
        .unpack(destination)
        .context("extract cache archive")
}

fn create_archive(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(destination)?;
    let bounded = BoundedWriter {
        inner: file,
        written: 0,
    };
    let encoder = zstd::Encoder::new(bounded, 3).context("create compressed cache")?;
    let mut archive = tar::Builder::new(encoder);
    archive.follow_symlinks(false);
    archive
        .append_dir_all(".", source)
        .context("archive cache directory")?;
    let encoder = archive.into_inner()?;
    encoder.finish()?;
    Ok(())
}

fn file_identity(path: &Path) -> anyhow::Result<(u64, String)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((size, hex::encode(hasher.finalize())))
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

struct BoundedWriter<W> {
    inner: W,
    written: u64,
}
impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.written.saturating_add(bytes.len() as u64) > MAX_CACHE_BYTES {
            return Err(std::io::Error::other("cache archive exceeds 10 GiB"));
        }
        let written = self.inner.write(bytes)?;
        self.written += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
