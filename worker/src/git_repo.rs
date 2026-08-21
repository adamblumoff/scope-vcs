use scope_git::{DEFAULT_GIT_BRANCH, GitStorageLimits};
use scope_git_process::{ProcessLimits, run as run_process};
use scope_object_store::{ObjectStore, source_blob_bytes_bounded};
use scope_postgres::db::GitCompactionCandidate;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

pub(crate) struct CompactedPack {
    pub(crate) bytes: Vec<u8>,
    pub(crate) metrics: CompactionPackMetrics,
}

pub(crate) struct CompactionPackMetrics {
    pub(crate) source_span_count: usize,
    pub(crate) source_pack_bytes: usize,
    pub(crate) predecessor_pack_bytes: usize,
    pub(crate) compacted_bytes: usize,
    pub(crate) init_ms: u64,
    pub(crate) download_ms: u64,
    pub(crate) index_ms: u64,
    pub(crate) update_ref_ms: u64,
    pub(crate) connectivity_check_ms: u64,
    pub(crate) pack_ms: u64,
    pub(crate) total_ms: u64,
}

pub(crate) fn build_compacted_pack(
    object_store: &dyn ObjectStore,
    candidate: &GitCompactionCandidate,
    storage_limits: GitStorageLimits,
    timeout: Duration,
) -> anyhow::Result<CompactedPack> {
    let total_started = Instant::now();
    let repo = TemporaryGitRepo::new()?;
    let init_started = Instant::now();
    run_git(
        None,
        &["init", "--bare", repo.path.to_string_lossy().as_ref()],
        None,
        timeout,
        storage_limits.max_object_bytes(),
    )?;
    let init_ms = elapsed_ms(init_started);
    let mut download = Duration::ZERO;
    let mut index = Duration::ZERO;
    let mut source_pack_bytes = 0usize;
    let mut predecessor_pack_bytes = 0usize;
    for (is_predecessor, span) in candidate
        .predecessor
        .iter()
        .map(|span| (true, span))
        .chain(candidate.spans.iter().map(|span| (false, span)))
    {
        let download_started = Instant::now();
        let bytes = source_blob_bytes_bounded(
            object_store,
            &span.object,
            storage_limits.max_object_bytes(),
        )
        .map_err(|error| anyhow::anyhow!(error.message))?;
        download += download_started.elapsed();
        if is_predecessor {
            predecessor_pack_bytes = predecessor_pack_bytes.saturating_add(bytes.len());
        } else {
            source_pack_bytes = source_pack_bytes.saturating_add(bytes.len());
        }
        let index_started = Instant::now();
        run_git(
            Some(&repo.path),
            &["index-pack", "--stdin"],
            Some(bytes),
            timeout,
            storage_limits.max_object_bytes(),
        )?;
        index += index_started.elapsed();
    }
    let compacted_head = candidate
        .spans
        .last()
        .ok_or_else(|| anyhow::anyhow!("Git compaction candidate has no pack spans"))?
        .head_oid
        .as_str();
    let compacted_base = candidate
        .spans
        .first()
        .ok_or_else(|| anyhow::anyhow!("Git compaction candidate has no pack spans"))?
        .base_oid
        .as_deref();
    match (compacted_base, candidate.predecessor.as_ref()) {
        (None, None) => {}
        (Some(base_oid), Some(predecessor)) if predecessor.head_oid == base_oid => {}
        _ => anyhow::bail!("Git compaction candidate has an invalid predecessor boundary"),
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
    )?;
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
    )?;
    let connectivity_check_ms = elapsed_ms(connectivity_started);
    let pack_started = Instant::now();
    let bytes = run_git(
        Some(&repo.path),
        &["pack-objects", "--revs", "--stdout"],
        Some(revisions.into_bytes()),
        timeout,
        storage_limits.max_object_bytes(),
    )?;
    let pack_ms = elapsed_ms(pack_started);
    Ok(CompactedPack {
        metrics: CompactionPackMetrics {
            source_span_count: candidate.spans.len(),
            source_pack_bytes,
            predecessor_pack_bytes,
            compacted_bytes: bytes.len(),
            init_ms,
            download_ms: duration_ms(download),
            index_ms: duration_ms(index),
            update_ref_ms,
            connectivity_check_ms,
            pack_ms,
            total_ms: elapsed_ms(total_started),
        },
        bytes,
    })
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
    fn new() -> anyhow::Result<Self> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| anyhow::anyhow!("creating compaction path: {error}"))?;
        let path = std::env::temp_dir().join(format!(
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
    use scope_domain::store::{DEFAULT_GIT_FILE_MODE, GitPackSpan};
    use scope_git_process::ProcessError;
    use scope_object_store::{ContentObjectKind, MemoryObjectStore, put_content_object};

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

    fn span(
        store: &MemoryObjectStore,
        first_sequence: u64,
        last_sequence: u64,
        tier: u32,
        base_oid: Option<String>,
        head_oid: String,
        pack: Vec<u8>,
    ) -> GitPackSpan {
        let mut object = put_content_object(store, ContentObjectKind::GitSegment, &pack).unwrap();
        object.git_oid = head_oid.clone();
        object.git_file_mode = DEFAULT_GIT_FILE_MODE.to_string();
        GitPackSpan {
            first_sequence,
            last_sequence,
            geometric_tier: tier,
            base_oid,
            head_oid,
            object,
        }
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

    #[test]
    fn interior_compaction_uses_its_predecessor_as_a_history_boundary() {
        let source = TemporaryGitRepo::new().unwrap();
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

        let store = MemoryObjectStore::new();
        let predecessor = span(
            &store,
            1,
            2,
            1,
            None,
            head_2.clone(),
            make_pack(&source.path, &head_2, None),
        );
        let selected = vec![
            span(
                &store,
                3,
                3,
                0,
                Some(head_2.clone()),
                head_3.clone(),
                make_pack(&source.path, &head_3, Some(&head_2)),
            ),
            span(
                &store,
                4,
                4,
                0,
                Some(head_3.clone()),
                head_4.clone(),
                make_pack(&source.path, &head_4, Some(&head_3)),
            ),
        ];
        let candidate = GitCompactionCandidate {
            repo_id: "owner/repo".to_string(),
            owner: "owner".to_string(),
            name: "repo".to_string(),
            predecessor: Some(predecessor),
            spans: selected,
        };

        let compacted = build_compacted_pack(
            &store,
            &candidate,
            GitStorageLimits::new(1024 * 1024).unwrap(),
            Duration::from_secs(2),
        )
        .unwrap();

        let result = TemporaryGitRepo::new().unwrap();
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
            Some(compacted.bytes),
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
}
