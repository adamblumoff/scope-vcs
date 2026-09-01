use scope_git::{DEFAULT_GIT_BRANCH, GitStorageLimits};
use scope_git_process::{
    ProcessError, ProcessLimits, StreamingProcessError, configure_process_group,
    run as run_process, run_with_stdout,
};
use scope_git_storage::{
    GitSegmentReservation, GitSegmentRestoreSource, GitSegmentRestoreTimings, GitSegmentStore,
    StagedGitSegment,
};
use scope_postgres::db::GitCompactionCandidate;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::AsyncReadExt;

pub(crate) struct CompactedPack {
    pub(crate) staged: StagedGitSegment,
    pub(crate) metrics: CompactionPackMetrics,
}

#[derive(Debug)]
pub(crate) struct CompactedPackFailure {
    pub(crate) error: anyhow::Error,
}

pub(crate) struct CompactionPackMetrics {
    pub(crate) source_span_count: usize,
    pub(crate) source_pack_bytes: usize,
    pub(crate) predecessor_pack_bytes: usize,
    pub(crate) compacted_bytes: usize,
    pub(crate) local_restore_count: usize,
    pub(crate) remote_restore_count: usize,
    pub(crate) init_ms: u64,
    pub(crate) download_ms: u64,
    pub(crate) index_ms: u64,
    pub(crate) update_ref_ms: u64,
    pub(crate) connectivity_check_ms: u64,
    pub(crate) pack_ms: u64,
    pub(crate) total_ms: u64,
}

pub(crate) async fn build_compacted_pack(
    segment_store: Arc<GitSegmentStore>,
    candidate: &GitCompactionCandidate,
    reservation: GitSegmentReservation,
    storage_limits: GitStorageLimits,
    timeout: Duration,
    data_dir: PathBuf,
) -> Result<CompactedPack, CompactedPackFailure> {
    build_compacted_pack_inner(
        segment_store,
        candidate,
        reservation,
        storage_limits,
        timeout,
        data_dir,
    )
    .await
}

async fn build_compacted_pack_inner(
    segment_store: Arc<GitSegmentStore>,
    candidate: &GitCompactionCandidate,
    reservation: GitSegmentReservation,
    storage_limits: GitStorageLimits,
    timeout: Duration,
    data_dir: PathBuf,
) -> Result<CompactedPack, CompactedPackFailure> {
    let total_started = Instant::now();
    let repo = TemporaryGitRepo::new(&data_dir)?;
    let init_started = Instant::now();
    run_git(
        None,
        &["init", "--bare", repo.path.to_string_lossy().as_ref()],
        None,
        timeout,
        storage_limits.max_object_bytes(),
    )
    .map_err(CompactedPackFailure::from)?;
    let init_ms = elapsed_ms(init_started);
    let mut download = Duration::ZERO;
    let mut index = Duration::ZERO;
    let mut source_pack_bytes = 0usize;
    let mut predecessor_pack_bytes = 0usize;
    let mut local_restore_count = 0usize;
    let mut remote_restore_count = 0usize;
    for (is_predecessor, span) in candidate
        .predecessor
        .iter()
        .map(|span| (true, span))
        .chain(candidate.spans.iter().map(|span| (false, span)))
    {
        if span.segment.plaintext_bytes
            > u64::try_from(storage_limits.max_object_bytes()).unwrap_or(u64::MAX)
        {
            return Err(CompactedPackFailure::from(anyhow::anyhow!(
                "Git segment {} exceeds the configured object byte limit",
                span.segment.segment_id
            )));
        }
        let index_started = Instant::now();
        let restore = index_git_segment(
            segment_store.as_ref(),
            &candidate.repo_id,
            &span.segment,
            &repo.path,
            timeout,
        )
        .await
        .map_err(CompactedPackFailure::from)?;
        match restore.source {
            GitSegmentRestoreSource::Local => local_restore_count += 1,
            GitSegmentRestoreSource::Remote => remote_restore_count += 1,
        }
        download += restore.total;
        index += index_started.elapsed().saturating_sub(restore.total);
        let bytes = usize::try_from(span.segment.plaintext_bytes).unwrap_or(usize::MAX);
        if is_predecessor {
            predecessor_pack_bytes = predecessor_pack_bytes.saturating_add(bytes);
        } else {
            source_pack_bytes = source_pack_bytes.saturating_add(bytes);
        }
    }
    let compacted_head = candidate
        .spans
        .last()
        .ok_or_else(|| {
            CompactedPackFailure::from(anyhow::anyhow!(
                "Git compaction candidate has no pack spans"
            ))
        })?
        .head_oid
        .as_str();
    let compacted_base = candidate
        .spans
        .first()
        .ok_or_else(|| {
            CompactedPackFailure::from(anyhow::anyhow!(
                "Git compaction candidate has no pack spans"
            ))
        })?
        .base_oid
        .as_deref();
    match (compacted_base, candidate.predecessor.as_ref()) {
        (None, None) => {}
        (Some(base_oid), Some(predecessor)) if predecessor.head_oid == base_oid => {}
        _ => {
            return Err(CompactedPackFailure::from(anyhow::anyhow!(
                "Git compaction candidate has an invalid predecessor boundary"
            )));
        }
    }
    let update_ref_started = Instant::now();
    run_git(
        Some(&repo.path),
        &[
            "update-ref",
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            compacted_head,
        ],
        None,
        timeout,
        storage_limits.max_object_bytes(),
    )
    .map_err(CompactedPackFailure::from)?;
    let update_ref_ms = elapsed_ms(update_ref_started);
    let revisions = match compacted_base {
        Some(base_oid) => format!("{compacted_head}\n^{base_oid}\n"),
        None => format!("{compacted_head}\n"),
    };
    let connectivity_started = Instant::now();
    run_git(
        Some(&repo.path),
        &[
            "rev-list",
            "--objects",
            "--missing=error",
            "--quiet",
            "--stdin",
        ],
        Some(revisions.as_bytes().to_vec()),
        timeout,
        storage_limits.max_object_bytes(),
    )
    .map_err(CompactedPackFailure::from)?;
    let connectivity_check_ms = elapsed_ms(connectivity_started);
    let pack_started = Instant::now();
    let staged = ingest_compacted_pack(
        Arc::clone(&segment_store),
        &candidate.repo_id,
        reservation,
        &repo.path,
        revisions.into_bytes(),
        timeout,
        storage_limits.max_object_bytes(),
    )
    .await?;
    let pack_ms = elapsed_ms(pack_started);
    let compacted_bytes = usize::try_from(staged.segment.plaintext_bytes).unwrap_or(usize::MAX);
    Ok(CompactedPack {
        metrics: CompactionPackMetrics {
            source_span_count: candidate.spans.len(),
            source_pack_bytes,
            predecessor_pack_bytes,
            compacted_bytes,
            local_restore_count,
            remote_restore_count,
            init_ms,
            download_ms: duration_ms(download),
            index_ms: duration_ms(index),
            update_ref_ms,
            connectivity_check_ms,
            pack_ms,
            total_ms: elapsed_ms(total_started),
        },
        staged,
    })
}

impl From<anyhow::Error> for CompactedPackFailure {
    fn from(error: anyhow::Error) -> Self {
        Self { error }
    }
}

async fn index_git_segment(
    segment_store: &GitSegmentStore,
    repository_id: &str,
    segment: &scope_domain::repository::git::GitSegmentRef,
    repo: &Path,
    timeout: Duration,
) -> anyhow::Result<GitSegmentRestoreTimings> {
    let mut command = tokio::process::Command::new("git");
    command
        .arg("--git-dir")
        .arg(repo)
        .args(["index-pack", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_process_group(command.as_std_mut());
    let mut child = command.spawn()?;
    let process_id = child.id();
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("opening git index-pack stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("opening git index-pack stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("opening git index-pack stderr"))?;
    let stdout_task = tokio::spawn(read_git_pipe(stdout, 64 * 1024));
    let stderr_task = tokio::spawn(read_git_pipe(stderr, 64 * 1024));
    let operation = async {
        let restore = segment_store.restore_to_prefer_local(repository_id, segment, stdin);
        let wait = child.wait();
        let (restore, status) = tokio::join!(restore, wait);
        Ok::<_, anyhow::Error>((restore?, status?))
    };
    let (restore, status) = match tokio::time::timeout(timeout, operation).await {
        Ok(result) => result?,
        Err(_) => {
            terminate_git_child(&mut child, process_id).await;
            return Err(anyhow::Error::new(ProcessError::TimedOut {
                action: "git index-pack --stdin".to_string(),
                timeout_ms: timeout.as_millis(),
                diagnostic: String::new(),
            }));
        }
    };
    let _stdout = stdout_task.await??;
    let stderr = stderr_task.await??;
    if !status.success() {
        anyhow::bail!(
            "git index-pack --stdin failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        );
    }
    Ok(restore)
}

async fn read_git_pipe(
    pipe: impl tokio::io::AsyncRead + Unpin,
    max_bytes: usize,
) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > max_bytes {
        return Err(std::io::Error::other(
            "Git process diagnostic output exceeded its byte limit",
        ));
    }
    Ok(bytes)
}

async fn terminate_git_child(child: &mut tokio::process::Child, process_id: Option<u32>) {
    if let Some(process_id) = process_id {
        scope_git_process::kill_process_group(process_id);
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn ingest_compacted_pack(
    segment_store: Arc<GitSegmentStore>,
    repository_id: &str,
    reservation: GitSegmentReservation,
    repo: &Path,
    revisions: Vec<u8>,
    timeout: Duration,
    max_bytes: usize,
) -> Result<StagedGitSegment, CompactedPackFailure> {
    let repository_id = repository_id.to_string();
    let repo = repo.to_path_buf();
    let runtime = tokio::runtime::Handle::current();
    let output = tokio::task::spawn_blocking(move || {
        let mut command = Command::new("git");
        command
            .arg("--git-dir")
            .arg(repo)
            .args(["pack-objects", "--revs", "--stdout"]);
        run_with_stdout(
            &mut command,
            Some(revisions),
            ProcessLimits::new(timeout),
            "git pack-objects --revs --stdout",
            move |stdout, cancellation| {
                runtime.block_on(segment_store.ingest_reserved_blocking_reader_cancellable(
                    &repository_id,
                    reservation,
                    stdout,
                    max_bytes as u64,
                    cancellation,
                ))
            },
        )
    })
    .await
    .map_err(|error| {
        CompactedPackFailure::from(anyhow::anyhow!("Git compaction task failed: {error}"))
    })?;
    let output = match output {
        Ok(output) => output,
        Err(StreamingProcessError::Process(error)) => {
            return Err(CompactedPackFailure::from(anyhow::Error::new(error)));
        }
        Err(StreamingProcessError::Consumer(
            scope_git_storage::GitStorageError::PlaintextLimitExceeded { .. },
        )) => {
            return Err(CompactedPackFailure::from(anyhow::Error::new(
                ProcessError::StdoutLimitExceeded {
                    action: "git pack-objects --revs --stdout".to_string(),
                    max_stdout_bytes: max_bytes,
                    diagnostic: String::new(),
                },
            )));
        }
        Err(StreamingProcessError::Consumer(error)) => {
            return Err(CompactedPackFailure::from(anyhow::Error::new(error)));
        }
    };
    if !output.status.success() {
        return Err(CompactedPackFailure::from(anyhow::anyhow!(
            "git pack-objects --revs --stdout failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.value)
}

fn elapsed_ms(started: Instant) -> u64 {
    duration_ms(started.elapsed())
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn run_git(
    git_dir: Option<&Path>,
    args: &[&str],
    input: Option<Vec<u8>>,
    timeout: Duration,
    max_stdout_bytes: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut command = Command::new("git");
    if let Some(git_dir) = git_dir {
        command.arg("--git-dir").arg(git_dir);
    }
    command.args(args);
    let output = run_process(
        &mut command,
        input,
        ProcessLimits::new(timeout).with_max_stdout_bytes(max_stdout_bytes),
        &format!("git {}", args.join(" ")),
    )
    .map_err(anyhow::Error::from)?;
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
}

impl TemporaryGitRepo {
    fn new(data_dir: &Path) -> anyhow::Result<Self> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| anyhow::anyhow!("creating compaction path: {error}"))?;
        let root = data_dir.join("git-compaction");
        fs::create_dir_all(&root)?;
        let path = root.join(format!(
            "scope-git-compact-{}-{}",
            std::process::id(),
            hex::encode(random)
        ));
        Ok(Self { path })
    }
}

impl Drop for TemporaryGitRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::repository::git::GitPackSpan;
    use scope_git_process::ProcessError;
    use scope_git_storage::{GitSegmentStoreConfig, MemoryMultipartStore, SegmentEncryptionKey};
    use std::io::Cursor;

    fn oid(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).unwrap().trim().to_string()
    }

    fn make_commit(repo: &Path, tree: &str, parent: Option<&str>, message: &str) -> String {
        let mut args = vec![
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope@test.invalid",
            "commit-tree",
            tree,
        ];
        if let Some(parent) = parent {
            args.extend(["-p", parent]);
        }
        oid(run_git(
            Some(repo),
            &args,
            Some(format!("{message}\n").into_bytes()),
            Duration::from_secs(2),
            1024,
        )
        .unwrap())
    }

    fn make_pack(repo: &Path, head: &str, base: Option<&str>) -> Vec<u8> {
        let revisions = match base {
            Some(base) => format!("{head}\n^{base}\n"),
            None => format!("{head}\n"),
        };
        run_git(
            Some(repo),
            &["pack-objects", "--revs", "--stdout"],
            Some(revisions.into_bytes()),
            Duration::from_secs(2),
            1024 * 1024,
        )
        .unwrap()
    }

    async fn span(
        store: &GitSegmentStore,
        repository_id: &str,
        sequences: (u64, u64),
        tier: u32,
        boundary: (Option<String>, String),
        pack: Vec<u8>,
    ) -> GitPackSpan {
        let staged = store
            .ingest_blocking_reader(repository_id, Cursor::new(pack), u64::MAX)
            .await
            .unwrap();
        GitPackSpan {
            first_sequence: sequences.0,
            last_sequence: sequences.1,
            geometric_tier: tier,
            base_oid: boundary.0,
            head_oid: boundary.1,
            segment: staged.segment,
        }
    }

    fn segment_store(local_root: &Path) -> Arc<GitSegmentStore> {
        let mut config = GitSegmentStoreConfig::new(local_root);
        config.chunk_bytes = 1024;
        config.multipart_part_bytes = 1024;
        Arc::new(
            GitSegmentStore::new(
                Arc::new(MemoryMultipartStore::default()),
                SegmentEncryptionKey::new("test", [7_u8; 32]).unwrap(),
                config,
            )
            .unwrap(),
        )
    }

    #[test]
    fn worker_git_output_obeys_exact_byte_limit() {
        let exact = run_git(
            None,
            &["hash-object", "--stdin"],
            Some(b"content".to_vec()),
            Duration::from_secs(1),
            41,
        )
        .unwrap();
        assert_eq!(exact.len(), 41);

        let error = run_git(
            None,
            &["hash-object", "--stdin"],
            Some(b"content".to_vec()),
            Duration::from_secs(1),
            40,
        )
        .unwrap_err();
        assert!(
            error
                .downcast_ref::<ProcessError>()
                .is_some_and(ProcessError::is_stdout_limit)
        );
    }

    #[tokio::test]
    async fn interior_compaction_uses_its_predecessor_as_a_history_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let source = TemporaryGitRepo::new(temp.path()).unwrap();
        run_git(
            None,
            &["init", "--bare", source.path.to_string_lossy().as_ref()],
            None,
            Duration::from_secs(2),
            1024,
        )
        .unwrap();
        let tree = oid(run_git(
            Some(&source.path),
            &["mktree"],
            Some(Vec::new()),
            Duration::from_secs(2),
            1024,
        )
        .unwrap());
        let head_1 = make_commit(&source.path, &tree, None, "one");
        let head_2 = make_commit(&source.path, &tree, Some(&head_1), "two");
        let head_3 = make_commit(&source.path, &tree, Some(&head_2), "three");
        let head_4 = make_commit(&source.path, &tree, Some(&head_3), "four");

        let repository_id = "owner/repo";
        let store = segment_store(&temp.path().join("segments"));
        let predecessor = span(
            store.as_ref(),
            repository_id,
            (1, 2),
            1,
            (None, head_2.clone()),
            make_pack(&source.path, &head_2, None),
        )
        .await;
        let selected = vec![
            span(
                store.as_ref(),
                repository_id,
                (3, 3),
                0,
                (Some(head_2.clone()), head_3.clone()),
                make_pack(&source.path, &head_3, Some(&head_2)),
            )
            .await,
            span(
                store.as_ref(),
                repository_id,
                (4, 4),
                0,
                (Some(head_3.clone()), head_4.clone()),
                make_pack(&source.path, &head_4, Some(&head_3)),
            )
            .await,
        ];
        for selected_span in &selected {
            store
                .cleanup_local(repository_id, &selected_span.segment.segment_id)
                .await
                .unwrap();
        }
        let candidate = GitCompactionCandidate {
            repo_id: repository_id.to_string(),
            owner: "owner".to_string(),
            name: "repo".to_string(),
            predecessor: Some(predecessor),
            spans: selected,
        };

        let reservation = store.reserve(repository_id).unwrap();
        let compacted = build_compacted_pack(
            Arc::clone(&store),
            &candidate,
            reservation,
            GitStorageLimits::new(1024 * 1024).unwrap(),
            Duration::from_secs(2),
            temp.path().to_path_buf(),
        )
        .await
        .unwrap();
        assert_eq!(compacted.metrics.local_restore_count, 1);
        assert_eq!(compacted.metrics.remote_restore_count, 2);
        let compacted_bytes = fs::read(compacted.staged.local_pack_path()).unwrap();

        let result = TemporaryGitRepo::new(temp.path()).unwrap();
        run_git(
            None,
            &["init", "--bare", result.path.to_string_lossy().as_ref()],
            None,
            Duration::from_secs(2),
            1024,
        )
        .unwrap();
        run_git(
            Some(&result.path),
            &["index-pack", "--stdin"],
            Some(compacted_bytes),
            Duration::from_secs(2),
            1024,
        )
        .unwrap();
        run_git(
            Some(&result.path),
            &["cat-file", "-e", &format!("{head_4}^{{commit}}")],
            None,
            Duration::from_secs(2),
            1,
        )
        .unwrap();
        assert!(
            run_git(
                Some(&result.path),
                &["cat-file", "-e", &head_2],
                None,
                Duration::from_secs(2),
                1,
            )
            .is_err(),
            "the compacted range must not absorb its predecessor"
        );
    }

    #[test]
    fn temporary_git_repositories_stay_under_the_worker_data_directory() {
        let temp = tempfile::tempdir().unwrap();
        let repo = TemporaryGitRepo::new(temp.path()).unwrap();

        assert!(repo.path.starts_with(temp.path().join("git-compaction")));
    }
}
