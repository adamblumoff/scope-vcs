use scope_core::db::MetadataStore;
use scope_core::{
    config::{
        DEFAULT_GIT_COMPACTION_SEGMENTS, SCOPE_DATA_DIR_ENV, SCOPE_OBJECT_STORE_ENV, non_empty_env,
    },
    db::GitCompactionCandidate,
    domain::store::{GitHead, GitSegment},
    git_segments::GitSegmentManifest,
    object_store::{
        EncryptedObjectStore, FileObjectStore, ObjectStore, S3ObjectStore, put_repo_object,
        source_blob_bytes,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const DEFAULT_BATCH_SIZE: usize = 10;
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;
const DEFAULT_SCHEMA_WAIT_SECS: u64 = 300;
const SCHEMA_WAIT_RETRY_SECS: u64 = 2;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "worker=info,scope_core=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    run().await
}

async fn run() -> anyhow::Result<()> {
    let settings = WorkerSettings::from_env()?;
    tracing::info!(
        worker_id = %settings.worker_id,
        batch_size = settings.batch_size,
        poll_interval_ms = settings.poll_interval.as_millis(),
        schema_wait_secs = settings.schema_wait_timeout.as_secs(),
        git_compaction_segments = settings.git_compaction_segments,
        "starting worker"
    );

    let metadata = MetadataStore::connect_worker_from_env_with_schema_wait(
        settings.schema_wait_timeout,
        Duration::from_secs(SCHEMA_WAIT_RETRY_SECS),
    )
    .await?;
    metadata
        .readiness_check()
        .await
        .map_err(|error| anyhow::anyhow!("metadata readiness check failed: {}", error.message))?;
    let object_store = object_store_from_env(&settings.data_dir)?;

    let mut next_compaction_attempt = Instant::now();
    loop {
        let summary = metadata
            .run_ready_outbox_jobs(&settings.worker_id, settings.batch_size)
            .await
            .map_err(|error| anyhow::anyhow!("running outbox jobs: {}", error.message))?;
        if summary.claimed > 0 {
            tracing::info!(
                claimed = summary.claimed,
                completed = summary.completed,
                failed = summary.failed,
                "processed outbox jobs"
            );
        }
        if Instant::now() >= next_compaction_attempt {
            match compact_one_git_repository(
                &metadata,
                object_store.as_ref(),
                settings.git_compaction_segments,
            )
            .await
            {
                Ok(true) => tracing::info!("compacted Git segment chain"),
                Ok(false) => {}
                Err(error) => {
                    next_compaction_attempt = Instant::now() + Duration::from_secs(30);
                    tracing::error!(
                        error = %error,
                        retry_seconds = 30,
                        "Git compaction failed; continuing worker loop"
                    );
                }
            }
        }
        let orphan_summary = drain_orphan_objects(&metadata, object_store.as_ref()).await?;
        if orphan_summary.attempted > 0 {
            tracing::info!(
                attempted = orphan_summary.attempted,
                deleted = orphan_summary.deleted,
                retained = orphan_summary.retained,
                "processed orphan object jobs"
            );
        }

        if summary.claimed >= settings.batch_size {
            continue;
        }

        tokio::select! {
            () = shutdown_signal() => {
                tracing::info!("worker shutdown requested");
                return Ok(());
            }
            () = tokio::time::sleep(settings.poll_interval) => {}
        }
    }
}

async fn compact_one_git_repository(
    metadata: &MetadataStore,
    object_store: &dyn ObjectStore,
    minimum_segments: usize,
) -> anyhow::Result<bool> {
    let Some(candidate) = metadata
        .git_compaction_candidate(minimum_segments as u64)
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?
    else {
        return Ok(false);
    };
    let (new_head, new_segment) = match build_compacted_segment(object_store, &candidate) {
        Ok(compaction) => compaction,
        Err(failure) => {
            if !failure.orphan_objects.is_empty() {
                queue_or_delete_failed_compaction_objects(
                    metadata,
                    object_store,
                    &failure.orphan_objects,
                )
                .await?;
            }
            return Err(failure.error);
        }
    };
    let stored_objects = [new_segment.object.clone(), new_segment.manifest.clone()];
    match metadata
        .replace_git_segments_with_compaction(
            &candidate.repo_id,
            &candidate.head.manifest.object_key,
            new_head,
            new_segment,
        )
        .await
    {
        Ok(applied) => Ok(applied),
        Err(error) => {
            if let Err(queue_error) = metadata
                .queue_pending_source_blob_deletions(stored_objects.to_vec())
                .await
            {
                return Err(anyhow::anyhow!(
                    "persisting Git compaction may have committed: {}; cleanup queue failed, retaining objects for reconciliation: {}",
                    error.message,
                    queue_error.message
                ));
            }
            Err(anyhow::anyhow!(
                "persisting Git compaction failed: {}",
                error.message
            ))
        }
    }
}

async fn queue_or_delete_failed_compaction_objects(
    metadata: &MetadataStore,
    object_store: &dyn ObjectStore,
    objects: &[scope_core::domain::store::SourceBlob],
) -> anyhow::Result<()> {
    if let Err(queue_error) = metadata
        .queue_pending_source_blob_deletions(objects.to_vec())
        .await
    {
        let mut delete_errors = Vec::new();
        for object in objects {
            if let Err(error) = object_store.delete(&object.object_key) {
                delete_errors.push(format!("{}: {}", object.object_key, error.message));
            }
        }
        if !delete_errors.is_empty() {
            anyhow::bail!(
                "cleanup queue failed: {}; direct cleanup failed: {}",
                queue_error.message,
                delete_errors.join(", ")
            );
        }
    }
    Ok(())
}

struct CompactedSegmentBuildFailure {
    error: anyhow::Error,
    orphan_objects: Vec<scope_core::domain::store::SourceBlob>,
}

impl From<anyhow::Error> for CompactedSegmentBuildFailure {
    fn from(error: anyhow::Error) -> Self {
        Self {
            error,
            orphan_objects: Vec::new(),
        }
    }
}

fn build_compacted_segment(
    object_store: &dyn ObjectStore,
    candidate: &GitCompactionCandidate,
) -> Result<(GitHead, GitSegment), CompactedSegmentBuildFailure> {
    let repo = TemporaryGitRepo::new()?;
    run_git(None, &["init", "--bare", repo.path_string.as_str()], None)?;
    for segment in &candidate.segments {
        let bytes = source_blob_bytes(object_store, &segment.object)
            .map_err(|error| anyhow::anyhow!(error.message))?;
        run_git(
            Some(&repo.path_string),
            &["index-pack", "--stdin"],
            Some(bytes),
        )?;
    }
    run_git(
        Some(&repo.path_string),
        &[
            "update-ref",
            "refs/heads/main",
            candidate.head.head_oid.as_str(),
        ],
        None,
    )?;
    run_git(
        Some(&repo.path_string),
        &[
            "fsck",
            "--connectivity-only",
            candidate.head.head_oid.as_str(),
        ],
        None,
    )?;
    let pack = run_git(
        Some(&repo.path_string),
        &["pack-objects", "--revs", "--stdout"],
        Some(format!("{}\n", candidate.head.head_oid).into_bytes()),
    )?;
    let segment = put_repo_object(object_store, &candidate.repo_id, "git-segments", &pack)
        .map_err(|error| CompactedSegmentBuildFailure::from(anyhow::anyhow!(error.message)))?;
    let manifest = GitSegmentManifest::new(candidate.head.head_oid.clone(), None, segment.clone());
    let mut manifest_object = match manifest.encode().and_then(|bytes| {
        put_repo_object(object_store, &candidate.repo_id, "git-manifests", &bytes)
    }) {
        Ok(object) => object,
        Err(error) => {
            return Err(CompactedSegmentBuildFailure {
                error: anyhow::anyhow!(error.message),
                orphan_objects: vec![segment],
            });
        }
    };
    manifest_object.git_oid = candidate.head.head_oid.clone();
    Ok((
        GitHead {
            head_oid: candidate.head.head_oid.clone(),
            segment_sequence: 1,
            change_version: candidate.head.change_version,
            manifest: manifest_object.clone(),
        },
        GitSegment {
            sequence: 1,
            base_oid: None,
            head_oid: candidate.head.head_oid.clone(),
            object: segment,
            manifest: manifest_object,
        },
    ))
}

fn run_git(
    git_dir: Option<&str>,
    args: &[&str],
    input: Option<Vec<u8>>,
) -> anyhow::Result<Vec<u8>> {
    let mut command = Command::new("git");
    if let Some(git_dir) = git_dir {
        command.arg("--git-dir").arg(git_dir);
    }
    command
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    if let Some(input) = input {
        child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Git stdin unavailable"))?
            .write_all(&input)?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

struct TemporaryGitRepo {
    path: PathBuf,
    path_string: String,
}

impl TemporaryGitRepo {
    fn new() -> anyhow::Result<Self> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| anyhow::anyhow!("creating compaction path: {error}"))?;
        let path = std::env::temp_dir().join(format!(
            "scope-git-compact-{}-{}",
            std::process::id(),
            hex::encode(random)
        ));
        let path_string = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("compaction path is not UTF-8"))?
            .to_string();
        Ok(Self { path, path_string })
    }
}

impl Drop for TemporaryGitRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn object_store_from_env(data_dir: &std::path::Path) -> anyhow::Result<Arc<dyn ObjectStore>> {
    let raw: Arc<dyn ObjectStore> = match non_empty_env(SCOPE_OBJECT_STORE_ENV).as_deref() {
        Some("filesystem") => Arc::new(FileObjectStore::from_env(&data_dir.join("objects"))),
        Some(value) if value != "s3" => {
            anyhow::bail!("unsupported {SCOPE_OBJECT_STORE_ENV} value {value}")
        }
        _ => Arc::new(S3ObjectStore::from_env()?),
    };
    Ok(Arc::new(EncryptedObjectStore::from_env(raw)?))
}

#[derive(Default)]
struct OrphanDrainSummary {
    attempted: usize,
    deleted: usize,
    retained: usize,
}

async fn drain_orphan_objects(
    metadata: &MetadataStore,
    object_store: &dyn ObjectStore,
) -> anyhow::Result<OrphanDrainSummary> {
    let batch = metadata
        .source_blob_cleanup_batch()
        .await
        .map_err(|error| anyhow::anyhow!("claiming orphan object jobs: {}", error.message))?;
    let mut candidates = BTreeMap::new();
    for object in &batch.pending {
        if !batch.referenced_blob_keys.contains(&object.object_key) {
            candidates
                .entry(object.object_key.clone())
                .or_insert(object);
        }
    }
    let mut deleted = BTreeSet::new();
    let mut retained = Vec::new();
    for object in candidates.values() {
        match object_store.delete(&object.object_key) {
            Ok(()) => {
                deleted.insert(object.object_key.clone());
            }
            Err(error) => {
                tracing::warn!(
                    object_key = %object.object_key,
                    error = %error.message,
                    "failed to delete orphan object"
                );
                retained.push((*object).clone());
            }
        }
    }
    let summary = OrphanDrainSummary {
        attempted: candidates.len(),
        deleted: deleted.len(),
        retained: retained.len(),
    };
    metadata
        .finish_source_blob_cleanup(batch, &retained)
        .await
        .map_err(|error| anyhow::anyhow!("finishing orphan object jobs: {}", error.message))?;
    Ok(summary)
}

struct WorkerSettings {
    worker_id: String,
    batch_size: usize,
    poll_interval: Duration,
    schema_wait_timeout: Duration,
    git_compaction_segments: usize,
    data_dir: PathBuf,
}

impl WorkerSettings {
    fn from_env() -> anyhow::Result<Self> {
        let worker_id = std::env::var("SCOPE_WORKER_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(default_worker_id);
        let batch_size = parse_usize_env("SCOPE_WORKER_BATCH_SIZE", DEFAULT_BATCH_SIZE)?;
        let poll_interval_ms =
            parse_u64_env("SCOPE_WORKER_POLL_INTERVAL_MS", DEFAULT_POLL_INTERVAL_MS)?;
        let schema_wait_secs =
            parse_u64_env("SCOPE_WORKER_SCHEMA_WAIT_SECS", DEFAULT_SCHEMA_WAIT_SECS)?;
        let git_compaction_segments = parse_usize_env(
            "SCOPE_GIT_COMPACTION_SEGMENTS",
            DEFAULT_GIT_COMPACTION_SEGMENTS,
        )?;
        let data_dir = non_empty_env(SCOPE_DATA_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".scope"));
        Ok(Self {
            worker_id,
            batch_size: batch_size.max(1),
            poll_interval: Duration::from_millis(poll_interval_ms.max(100)),
            schema_wait_timeout: Duration::from_secs(schema_wait_secs.max(1)),
            git_compaction_segments: git_compaction_segments.max(2),
            data_dir,
        })
    }
}

fn parse_usize_env(name: &str, default: usize) -> anyhow::Result<usize> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .parse::<usize>()
            .map_err(|error| anyhow::anyhow!("{name} must be an integer: {error}")),
        _ => Ok(default),
    }
}

fn parse_u64_env(name: &str, default: u64) -> anyhow::Result<u64> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .parse::<u64>()
            .map_err(|error| anyhow::anyhow!("{name} must be an integer: {error}")),
        _ => Ok(default),
    }
}

fn default_worker_id() -> String {
    let host = std::env::var("RAILWAY_REPLICA_ID")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "local".to_string());
    format!("scope-worker-{host}-{}", std::process::id())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
