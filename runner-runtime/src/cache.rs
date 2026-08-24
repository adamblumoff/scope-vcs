use crate::api::{CacheDownloadError, RuntimeClient};
use anyhow::Context as _;
use scope_api_contract::{
    AttemptCacheFinalizationReport, AttemptCacheKeyMaterial, AttemptCachePreparationReport,
    ReportAttemptCacheFinalizationsRequest, ReportAttemptCachePreparationsRequest, RunJobResponse,
};
use scope_cache_contract::{
    CacheRestoreSource, CommitCacheUploadRequest, PrepareCacheUploadRequest,
    PrepareCacheUploadResponse, RestoreCacheRequest, RestoreCacheResponse,
};
use scope_cache_domain::{CacheDigest, MAX_CACHE_OBJECT_BYTES};
use scope_domain::runs::cache::{
    CacheColdReason, CacheFinalState, CacheIdentity, CacheKeyInputs, CacheNamespace, CachePlatform,
    CachePreparation,
};
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    time::Instant,
};

const MAX_CACHE_KEY_FILE_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) struct PreparedCache {
    exact_digest: String,
    compatibility_group_digest: String,
    path: PathBuf,
    exact_hit: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CacheFinalizationOutcome {
    Ready,
    Unchanged,
    Skipped {
        reason: CacheSkipReason,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CacheSkipReason {
    ArchiveFailed,
    ServiceUnavailable,
    UploadFailed,
    CommitFailed,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CacheFinalization {
    pub(crate) identity_digest: String,
    pub(crate) outcome: CacheFinalizationOutcome,
}

pub(crate) fn prepare_caches(
    client: &RuntimeClient,
    job: &RunJobResponse,
) -> anyhow::Result<Vec<PreparedCache>> {
    let workflow_path =
        scope_domain::runs::workflow::WorkflowPath::parse(job.workflow_path.clone())?;
    let job_key = scope_domain::runs::workflow::WorkflowJobId::parse(job.job_key.clone())?;
    let namespace = CacheNamespace::workflow(&workflow_path, &job_key);
    let mut identities = Vec::new();
    let mut key_material = Vec::new();
    for cache in job.definition.caches() {
        let compatibility_inputs_digest =
            digest_inputs(cache.compatibility_inputs(), job.definition.environment())?;
        let exact_inputs_digest =
            digest_inputs(cache.exact_inputs(), job.definition.environment())?;
        let identity = CacheIdentity::new(
            &job.repository_id,
            namespace.clone(),
            cache.clone(),
            CachePlatform::LinuxAmd64,
            &compatibility_inputs_digest,
            &exact_inputs_digest,
        )?;
        key_material.push(AttemptCacheKeyMaterial {
            cache_name: cache.as_str().to_string(),
            compatibility_inputs_digest,
            exact_inputs_digest,
        });
        identities.push(identity);
    }
    client.authorize_cache_keys(key_material)?;
    let mut prepared = Vec::new();
    let mut reports = Vec::new();
    for (cache, identity) in job.definition.caches().iter().zip(identities) {
        let started = Instant::now();
        let exact_digest = identity.exact_digest();
        let compatibility_group_digest = identity.compatibility_group_digest();
        let path = PathBuf::from(cache.mount_path());
        fs::create_dir_all(&path)
            .with_context(|| format!("create cache path {}", path.display()))?;
        let restore = restore_cache(client, &exact_digest, &compatibility_group_digest, &path)?;
        reports.push(AttemptCachePreparationReport {
            cache_name: cache.as_str().to_string(),
            identity_digest: exact_digest.clone(),
            preparation: restore.preparation,
            prepare_ms: elapsed_ms(started),
        });
        prepared.push(PreparedCache {
            exact_digest,
            compatibility_group_digest,
            path,
            exact_hit: restore.exact_hit,
        });
    }
    if let Err(error) =
        client.report_cache_preparations(&ReportAttemptCachePreparationsRequest { caches: reports })
    {
        eprintln!("runtime cache preparation reporting skipped: {error:#}");
    }
    Ok(prepared)
}

pub(crate) fn save_caches(
    client: &RuntimeClient,
    caches: &[PreparedCache],
) -> Vec<CacheFinalization> {
    let mut finalizations = Vec::with_capacity(caches.len());
    let mut reports = Vec::new();
    for cache in caches {
        let started = Instant::now();
        let outcome = save_cache(client, cache);
        if matches!(
            outcome,
            CacheFinalizationOutcome::Ready | CacheFinalizationOutcome::Unchanged
        ) {
            reports.push(AttemptCacheFinalizationReport {
                identity_digest: cache.exact_digest.clone(),
                final_state: CacheFinalState::Ready,
                finalize_ms: elapsed_ms(started),
            });
        }
        finalizations.push(CacheFinalization {
            identity_digest: cache.exact_digest.clone(),
            outcome,
        });
    }
    if !reports.is_empty()
        && let Err(error) = client
            .report_cache_finalizations(&ReportAttemptCacheFinalizationsRequest { caches: reports })
    {
        eprintln!("runtime cache finalization reporting skipped: {error:#}");
    }
    finalizations
}

fn restore_cache(
    client: &RuntimeClient,
    exact_digest: &str,
    compatibility_group_digest: &str,
    destination: &Path,
) -> anyhow::Result<CacheRestore> {
    let exact_identity_digest = CacheDigest::parse(exact_digest.to_string())?;
    let compatibility_group_digest = CacheDigest::parse(compatibility_group_digest.to_string())?;
    let session = match client.restore_cache(&RestoreCacheRequest {
        exact_identity_digest,
        compatibility_group_digest,
    }) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("runtime cache restore unavailable for {exact_digest}: {error:#}");
            return Ok(CacheRestore::cold(CacheColdReason::MetadataNotReady));
        }
    };
    let (source, url, checksum, size) = match session {
        RestoreCacheResponse::Hit {
            source,
            object_digest,
            size_bytes,
            download_url,
            ..
        } => (
            source,
            download_url,
            object_digest.as_str().to_string(),
            size_bytes,
        ),
        RestoreCacheResponse::Miss => {
            return Ok(CacheRestore::cold(CacheColdReason::MetadataMissing));
        }
    };
    let temp_dir = match tempfile::tempdir().context("create cache download directory") {
        Ok(temp_dir) => temp_dir,
        Err(error) => {
            eprintln!("runtime cache restore staging failed for {exact_digest}: {error:#}");
            return Ok(CacheRestore::cold(CacheColdReason::MetadataNotReady));
        }
    };
    let archive = temp_dir.path().join("cache.tar.zst");
    if let Err(error) = client.download_cache(&url, &archive, size, &checksum) {
        let reason = match error {
            CacheDownloadError::Transport(error) => {
                eprintln!("runtime cache restore transport failed for {exact_digest}: {error:#}");
                CacheColdReason::MetadataNotReady
            }
            CacheDownloadError::Invalid(error) => {
                eprintln!("runtime cache restore rejected for {exact_digest}: {error:#}");
                CacheColdReason::MetadataInvalid
            }
        };
        return Ok(CacheRestore::cold(reason));
    }
    if let Err(error) = extract_archive(&archive, destination) {
        reset_cache_directory(destination)?;
        eprintln!("runtime cache restore was corrupt for {exact_digest}: {error:#}");
        return Ok(CacheRestore::cold(CacheColdReason::MetadataInvalid));
    }
    Ok(CacheRestore {
        preparation: match source {
            CacheRestoreSource::Exact => CachePreparation::Exact,
            CacheRestoreSource::Compatible => CachePreparation::Compatible,
        },
        exact_hit: source == CacheRestoreSource::Exact,
    })
}

fn save_cache(client: &RuntimeClient, cache: &PreparedCache) -> CacheFinalizationOutcome {
    if cache.exact_hit {
        return CacheFinalizationOutcome::Unchanged;
    }
    let temp = match tempfile::NamedTempFile::new().context("create cache upload file") {
        Ok(temp) => temp,
        Err(error) => return skipped(CacheSkipReason::ArchiveFailed, error),
    };
    if let Err(error) = create_archive(&cache.path, temp.path()) {
        return skipped(CacheSkipReason::ArchiveFailed, error);
    }
    let (size_bytes, checksum_sha256) = match file_identity(temp.path()) {
        Ok(identity) => identity,
        Err(error) => return skipped(CacheSkipReason::ArchiveFailed, error),
    };
    let exact_identity_digest = match CacheDigest::parse(cache.exact_digest.clone()) {
        Ok(digest) => digest,
        Err(error) => return skipped(CacheSkipReason::ServiceUnavailable, error.into()),
    };
    let object_digest = match CacheDigest::parse(checksum_sha256.clone()) {
        Ok(digest) => digest,
        Err(error) => return skipped(CacheSkipReason::ArchiveFailed, error.into()),
    };
    let session = match client.prepare_cache_upload(&PrepareCacheUploadRequest {
        exact_identity_digest: exact_identity_digest.clone(),
        compatibility_group_digest: match CacheDigest::parse(
            cache.compatibility_group_digest.clone(),
        ) {
            Ok(digest) => digest,
            Err(error) => return skipped(CacheSkipReason::ServiceUnavailable, error.into()),
        },
        object_digest: object_digest.clone(),
        size_bytes,
    }) {
        Ok(session) => session,
        Err(error) => return skipped(CacheSkipReason::ServiceUnavailable, error),
    };
    match session {
        PrepareCacheUploadResponse::UseObject { .. } => CacheFinalizationOutcome::Ready,
        PrepareCacheUploadResponse::Upload {
            lease_id,
            upload_url,
            upload_headers,
            ..
        } => {
            if let Err(error) = client.upload_cache(&upload_url, &upload_headers, temp.path()) {
                return skipped(CacheSkipReason::UploadFailed, error);
            }
            if let Err(error) = client.commit_cache_upload(&CommitCacheUploadRequest {
                lease_id,
                object_digest,
                size_bytes,
            }) {
                return skipped(CacheSkipReason::CommitFailed, error);
            }
            CacheFinalizationOutcome::Ready
        }
    }
}

fn skipped(reason: CacheSkipReason, error: anyhow::Error) -> CacheFinalizationOutcome {
    CacheFinalizationOutcome::Skipped {
        reason,
        message: format!("{error:#}"),
    }
}

struct CacheRestore {
    preparation: CachePreparation,
    exact_hit: bool,
}

impl CacheRestore {
    fn cold(reason: CacheColdReason) -> Self {
        Self {
            preparation: CachePreparation::Cold { reason },
            exact_hit: false,
        }
    }
}

fn digest_inputs(
    inputs: &CacheKeyInputs,
    environment: &std::collections::BTreeMap<String, String>,
) -> anyhow::Result<String> {
    digest_inputs_at(inputs, environment, Path::new("."))
}

fn digest_inputs_at(
    inputs: &CacheKeyInputs,
    environment: &std::collections::BTreeMap<String, String>,
    root: &Path,
) -> anyhow::Result<String> {
    let mut digest = Sha256::new();
    digest.update(b"scope-cache-inputs-v1");
    for path in inputs.files() {
        update_component(&mut digest, "file");
        update_component(&mut digest, path);
        match open_key_file(root, path)? {
            Some((mut file, size)) => {
                update_component(&mut digest, "present");
                digest.update(size.to_be_bytes());
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    let read = file
                        .read(&mut buffer)
                        .with_context(|| format!("read cache key input {path}"))?;
                    if read == 0 {
                        break;
                    }
                    digest.update(&buffer[..read]);
                }
            }
            None => {
                update_component(&mut digest, "missing");
            }
        }
    }
    for name in inputs.environment() {
        update_component(&mut digest, "environment");
        update_component(&mut digest, name);
        match environment.get(name) {
            Some(value) => {
                update_component(&mut digest, "present");
                update_component(&mut digest, value);
            }
            None => update_component(&mut digest, "missing"),
        }
    }
    Ok(hex::encode(digest.finalize()))
}

fn open_key_file(root: &Path, relative: &str) -> anyhow::Result<Option<(fs::File, u64)>> {
    let mut path = root.to_path_buf();
    let mut components = Path::new(relative).components().peekable();
    while let Some(component) = components.next() {
        path.push(component);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect cache key input {}", path.display()));
            }
        };
        if components.peek().is_some() {
            if !metadata.file_type().is_dir() {
                anyhow::bail!(
                    "cache key input {} traverses a non-directory or symlink",
                    path.display()
                );
            }
        } else if !metadata.file_type().is_file() {
            anyhow::bail!("cache key input {} is not a regular file", path.display());
        }
    }
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("inspect cache key input {}", path.display()))?;
    if metadata.len() > MAX_CACHE_KEY_FILE_BYTES {
        anyhow::bail!(
            "cache key input {} exceeds {MAX_CACHE_KEY_FILE_BYTES} bytes",
            path.display()
        );
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .with_context(|| format!("open cache key input {}", path.display()))?;
    let opened_metadata = file.metadata()?;
    if !opened_metadata.is_file() {
        anyhow::bail!("cache key input {} is not a regular file", path.display());
    }
    if opened_metadata.len() > MAX_CACHE_KEY_FILE_BYTES {
        anyhow::bail!(
            "cache key input {} exceeds {MAX_CACHE_KEY_FILE_BYTES} bytes",
            path.display()
        );
    }
    Ok(Some((file, opened_metadata.len())))
}

fn update_component(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value.as_bytes());
}

fn reset_cache_directory(path: &Path) -> anyhow::Result<()> {
    fs::remove_dir_all(path)
        .with_context(|| format!("remove partial cache directory {}", path.display()))?;
    fs::create_dir_all(path).with_context(|| format!("recreate cache directory {}", path.display()))
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
    let bounded = BoundedWriter::new(file, MAX_CACHE_OBJECT_BYTES);
    let encoder = zstd::Encoder::new(bounded, 3).context("create compressed cache")?;
    let mut archive = tar::Builder::new(encoder);
    append_directory_contents(&mut archive, source, Path::new(""))?;
    let encoder = archive.into_inner()?;
    encoder.finish()?;
    Ok(())
}

fn append_directory_contents<W: Write>(
    archive: &mut tar::Builder<W>,
    source: &Path,
    relative: &Path,
) -> anyhow::Result<()> {
    let directory = source.join(relative);
    let mut entries = fs::read_dir(&directory)
        .with_context(|| format!("read cache directory {}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let relative_path = relative.join(entry.file_name());
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("read cache entry metadata {}", path.display()))?;
        let file_type = metadata.file_type();
        if file_type.is_dir() {
            let mut header = normalized_header(tar::EntryType::Directory, 0, 0o755)?;
            append_entry(archive, &mut header, &relative_path, std::io::empty())?;
            append_directory_contents(archive, source, &relative_path)?;
        } else if file_type.is_file() {
            let mode = if metadata.permissions().mode() & 0o111 == 0 {
                0o644
            } else {
                0o755
            };
            let file = fs::File::open(&path)
                .with_context(|| format!("open cache entry {}", path.display()))?;
            let mut header = normalized_header(tar::EntryType::Regular, metadata.len(), mode)?;
            append_entry(archive, &mut header, &relative_path, file)?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(&path)
                .with_context(|| format!("read cache symlink {}", path.display()))?;
            let mut header = normalized_header(tar::EntryType::Symlink, 0, 0o777)?;
            archive
                .append_link(&mut header, &relative_path, target)
                .with_context(|| format!("archive cache symlink {}", relative_path.display()))?;
        } else {
            anyhow::bail!(
                "cache entry {} has an unsupported file type",
                path.display()
            );
        }
    }
    Ok(())
}

fn normalized_header(
    entry_type: tar::EntryType,
    size: u64,
    mode: u32,
) -> anyhow::Result<tar::Header> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(entry_type);
    header.set_size(size);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_username("")?;
    header.set_groupname("")?;
    header.set_cksum();
    Ok(header)
}

fn append_entry<W: Write, R: Read>(
    archive: &mut tar::Builder<W>,
    header: &mut tar::Header,
    path: &Path,
    content: R,
) -> anyhow::Result<()> {
    archive
        .append_data(header, path, content)
        .with_context(|| format!("archive cache entry {}", path.display()))
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
    max_bytes: u64,
}

impl<W> BoundedWriter<W> {
    fn new(inner: W, max_bytes: u64) -> Self {
        Self {
            inner,
            written: 0,
            max_bytes,
        }
    }
}

impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.written.saturating_add(bytes.len() as u64) > self.max_bytes {
            return Err(std::io::Error::other(format!(
                "cache archive exceeds {} bytes",
                self.max_bytes
            )));
        }
        let written = self.inner.write(bytes)?;
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::BTreeMap,
        os::unix::fs::symlink,
        time::{Duration, SystemTime},
    };

    #[test]
    fn archives_are_identical_across_creation_order_and_metadata() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        populate_cache(first.path(), false, Duration::from_secs(10));
        populate_cache(second.path(), true, Duration::from_secs(20));
        let first_archive = tempfile::NamedTempFile::new().unwrap();
        let second_archive = tempfile::NamedTempFile::new().unwrap();

        create_archive(first.path(), first_archive.path()).unwrap();
        create_archive(second.path(), second_archive.path()).unwrap();

        let first_bytes = fs::read(first_archive.path()).unwrap();
        let second_bytes = fs::read(second_archive.path()).unwrap();
        assert_eq!(first_bytes, second_bytes);
        assert_eq!(
            file_identity(first_archive.path()).unwrap(),
            file_identity(second_archive.path()).unwrap()
        );
    }

    #[test]
    fn archives_have_sorted_paths_and_normalized_headers() {
        let source = tempfile::tempdir().unwrap();
        populate_cache(source.path(), true, Duration::from_secs(30));
        let output = tempfile::NamedTempFile::new().unwrap();
        create_archive(source.path(), output.path()).unwrap();

        let decoder = zstd::Decoder::new(fs::File::open(output.path()).unwrap()).unwrap();
        let mut archive = tar::Archive::new(decoder);
        let headers = archive
            .entries()
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let header = entry.header();
                (
                    entry.path().unwrap().into_owned(),
                    header.mode().unwrap(),
                    header.uid().unwrap(),
                    header.gid().unwrap(),
                    header.mtime().unwrap(),
                )
            })
            .collect::<Vec<_>>();

        let paths = headers
            .iter()
            .map(|(path, ..)| path.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            ["bin", "bin/run", "data.txt", "run-link"]
                .map(PathBuf::from)
                .to_vec()
        );
        assert!(
            headers
                .iter()
                .all(|(_, _, uid, gid, mtime)| (*uid, *gid, *mtime) == (0, 0, 0))
        );
        let modes = headers
            .into_iter()
            .map(|(path, mode, ..)| (path, mode))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(modes[Path::new("bin")], 0o755);
        assert_eq!(modes[Path::new("bin/run")], 0o755);
        assert_eq!(modes[Path::new("data.txt")], 0o644);
        assert_eq!(modes[Path::new("run-link")], 0o777);
    }

    #[test]
    fn bounded_writer_accepts_the_limit_and_rejects_the_next_byte() {
        let mut writer = BoundedWriter::new(Vec::new(), 4);
        writer.write_all(b"four").unwrap();
        let error = writer.write_all(b"!").unwrap_err();
        assert_eq!(error.to_string(), "cache archive exceeds 4 bytes");
        assert_eq!(writer.written, 4);
        assert_eq!(MAX_CACHE_OBJECT_BYTES, 1024 * 1024 * 1024);
    }

    #[test]
    fn exact_hit_skips_archive_hash_and_upload() {
        let cache = PreparedCache {
            exact_digest: "a".repeat(64),
            compatibility_group_digest: "b".repeat(64),
            path: PathBuf::from("/path/that/does/not/exist"),
            exact_hit: true,
        };

        assert_eq!(
            save_cache(&RuntimeClient::disconnected_for_cache_tests(), &cache),
            CacheFinalizationOutcome::Unchanged
        );
    }

    #[test]
    fn cache_input_digest_distinguishes_missing_empty_content_and_environment() {
        let root = tempfile::tempdir().unwrap();
        let inputs = CacheKeyInputs::new(
            vec!["Cargo.lock".to_string()],
            vec!["RUSTUP_TOOLCHAIN".to_string()],
        )
        .unwrap();
        let mut environment =
            BTreeMap::from([("RUSTUP_TOOLCHAIN".to_string(), "1.98.0".to_string())]);
        let missing = digest_inputs_at(&inputs, &environment, root.path()).unwrap();
        fs::write(root.path().join("Cargo.lock"), []).unwrap();
        let empty = digest_inputs_at(&inputs, &environment, root.path()).unwrap();
        fs::write(root.path().join("Cargo.lock"), b"lock").unwrap();
        let content = digest_inputs_at(&inputs, &environment, root.path()).unwrap();
        environment.insert("RUSTUP_TOOLCHAIN".to_string(), "1.99.0".to_string());
        let environment_changed = digest_inputs_at(&inputs, &environment, root.path()).unwrap();

        assert_ne!(missing, empty);
        assert_ne!(empty, content);
        assert_ne!(content, environment_changed);
    }

    #[test]
    fn cache_input_hashing_rejects_symlinks_and_directories() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("outside"), b"secret").unwrap();
        symlink("outside", root.path().join("linked")).unwrap();
        fs::create_dir(root.path().join("directory")).unwrap();
        fs::create_dir(root.path().join("outside-directory")).unwrap();
        fs::write(root.path().join("outside-directory/input"), b"secret").unwrap();
        symlink("outside-directory", root.path().join("linked-directory")).unwrap();
        assert!(open_key_file(root.path(), "linked").is_err());
        assert!(open_key_file(root.path(), "directory").is_err());
        assert!(open_key_file(root.path(), "linked-directory/input").is_err());

        let oversized = fs::File::create(root.path().join("oversized")).unwrap();
        oversized.set_len(MAX_CACHE_KEY_FILE_BYTES + 1).unwrap();
        assert!(open_key_file(root.path(), "oversized").is_err());
    }

    fn populate_cache(root: &Path, reverse: bool, modified_offset: Duration) {
        let files = if reverse {
            [("data.txt", "data"), ("bin/run", "#!/bin/sh\n")]
        } else {
            [("bin/run", "#!/bin/sh\n"), ("data.txt", "data")]
        };
        for (path, contents) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            let mode = if path.ends_with("run") { 0o755 } else { 0o644 };
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            let file = fs::File::options().write(true).open(path).unwrap();
            file.set_times(
                fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + modified_offset),
            )
            .unwrap();
        }
        symlink("bin/run", root.join("run-link")).unwrap();
    }
}
