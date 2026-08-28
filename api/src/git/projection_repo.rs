use crate::{
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::{
        cache::{GitDerivedCacheNamespace, GitRepoHandle},
        content::source_content_bytes_from_repo,
        upload::{git_command_output, git_process_output_with_timeout, truncated_git_stderr},
    },
    runtime_budgets::RuntimeBudgets,
    state::AppState,
};
use scope_domain::{
    content::is_supported_git_file_mode,
    content_ref::ContentRef,
    projection::{Projection, ProjectionMaterialization},
};
use scope_git::GitTreePath;
use sha1::{Digest, Sha1};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path as FsPath, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

const PROJECTION_CACHE_SEMANTICS_VERSION: &str = "shared-projection-view-v2-native-commits";
static PROJECTION_CACHE_ATTEMPT: AtomicU64 = AtomicU64::new(1);

struct ProjectionBuildArtifacts {
    repo: PathBuf,
    index: PathBuf,
}

impl Drop for ProjectionBuildArtifacts {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.repo);
        let _ = fs::remove_file(&self.index);
    }
}

fn projection_bare_repo_with_loader(
    cache_root: &FsPath,
    projection: &Projection,
    native_source_repo: Option<&FsPath>,
    load_content: impl Fn(&scope_domain::content::SourceBlob) -> Result<Vec<u8>, ApiError>,
) -> Result<PathBuf, ApiError> {
    let cache_key = projection_cache_key(projection);
    let repo_path = cache_root.join(format!("{cache_key}.git"));
    if repo_path
        .join("refs")
        .join("heads")
        .join(DEFAULT_GIT_BRANCH)
        .is_file()
    {
        return Ok(repo_path);
    }

    let attempt = PROJECTION_CACHE_ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let temp_path = cache_root.join(format!(
        "{cache_key}.{}.{}.tmp",
        std::process::id(),
        attempt
    ));
    let index_path = cache_root.join(format!(
        "{cache_key}.{}.{}.index",
        std::process::id(),
        attempt
    ));
    if temp_path.exists() {
        fs::remove_dir_all(&temp_path).map_err(ApiError::internal)?;
    }
    if index_path.exists() {
        fs::remove_file(&index_path).map_err(ApiError::internal)?;
    }
    let _artifacts = ProjectionBuildArtifacts {
        repo: temp_path.clone(),
        index: index_path.clone(),
    };

    git_command_output(
        Command::new("git")
            .arg("init")
            .arg("--bare")
            .arg(&temp_path),
        None,
    )?;
    git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&temp_path)
            .arg("symbolic-ref")
            .arg("HEAD")
            .arg(format!("refs/heads/{DEFAULT_GIT_BRANCH}")),
        None,
    )?;
    let mut visible_tree = BTreeMap::new();
    let mut parent_commit: Option<String> = None;
    let mut native_range: Option<NativeRangeState> = None;
    if projection.commits.is_empty() {
        let tree = write_projection_tree(&temp_path, &index_path, &visible_tree)?;
        parent_commit = Some(git_commit_tree(
            &temp_path,
            &tree,
            None,
            "Empty Scope projection\n",
        )?);
    }

    for projected in &projection.commits {
        if native_range
            .as_ref()
            .is_some_and(|range| range.logical_commit_id != projected.logical_commit_id)
        {
            finish_native_range(
                &temp_path,
                &index_path,
                &visible_tree,
                native_range.take().expect("native range checked"),
            )?;
        }

        for change in &projected.changes {
            let path = GitTreePath::from_scope_path(&change.path).map_err(ApiError::internal)?;
            match &change.new_content {
                Some(blob) => {
                    visible_tree.insert(
                        path,
                        ProjectionTreeFile {
                            bytes: load_content(blob)?,
                            git_file_mode: blob.git_file_mode.clone(),
                        },
                    );
                }
                None => {
                    visible_tree.remove(&path);
                }
            }
        }

        match &projected.materialization {
            ProjectionMaterialization::Generate => {
                if let Some(range) = native_range.take() {
                    finish_native_range(&temp_path, &index_path, &visible_tree, range)?;
                }
                let tree = write_projection_tree(&temp_path, &index_path, &visible_tree)?;
                let message = format!("{}\n", projected.message);
                parent_commit = Some(git_commit_tree(
                    &temp_path,
                    &tree,
                    parent_commit.as_deref(),
                    &message,
                )?);
            }
            ProjectionMaterialization::PreserveGitCommit {
                oid,
                parent_oids,
                tree_oid,
            } => {
                let source_repo = native_source_repo.ok_or_else(|| {
                    ApiError::internal_message(
                        "native public projection commit requires canonical Git storage",
                    )
                })?;
                if oid != &projected.projected_id {
                    return Err(ApiError::internal_message(
                        "native public projection identity does not match projected identity",
                    ));
                }
                if native_range.is_none() {
                    let base_oid = parent_commit
                        .as_deref()
                        .map(str::trim)
                        .ok_or_else(|| {
                            ApiError::internal_message(
                                "native public projection range requires an existing public base",
                            )
                        })?
                        .to_string();
                    native_range = Some(NativeRangeState {
                        logical_commit_id: projected.logical_commit_id.clone(),
                        base_oid,
                        seen_oids: BTreeSet::new(),
                        head_oid: oid.clone(),
                    });
                }
                let range = native_range.as_mut().expect("native range initialized");
                copy_and_verify_native_commit(
                    source_repo,
                    &temp_path,
                    &range.base_oid,
                    &range.seen_oids,
                    oid,
                    parent_oids,
                    tree_oid,
                )?;
                range.seen_oids.insert(oid.clone());
                range.head_oid.clone_from(oid);
                parent_commit = Some(oid.clone());
            }
        }
    }

    if let Some(range) = native_range.take() {
        finish_native_range(&temp_path, &index_path, &visible_tree, range)?;
    }

    let commit = parent_commit.ok_or_else(|| ApiError::internal_message("missing Git commit"))?;
    if let Some(expected_head) =
        scope_git::projection_head_oid(projection).map_err(ApiError::internal)?
        && commit.trim() != expected_head
    {
        return Err(ApiError::internal_message(format!(
            "materialized projection head {} does not match canonical identity {expected_head}",
            commit.trim()
        )));
    }
    git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&temp_path)
            .arg("update-ref")
            .arg(format!("refs/heads/{DEFAULT_GIT_BRANCH}"))
            .arg(commit.trim()),
        None,
    )?;

    match fs::rename(&temp_path, &repo_path) {
        Ok(()) => Ok(repo_path),
        Err(error) if repo_path.exists() => {
            let _ = fs::remove_dir_all(&temp_path);
            tracing::debug!(%error, path = %repo_path.display(), "using concurrently-created Git projection cache");
            Ok(repo_path)
        }
        Err(error) => Err(ApiError::internal(error)),
    }
}

struct NativeRangeState {
    logical_commit_id: String,
    base_oid: String,
    seen_oids: BTreeSet<String>,
    head_oid: String,
}

fn copy_and_verify_native_commit(
    source_repo: &FsPath,
    target_repo: &FsPath,
    range_base_oid: &str,
    seen_oids: &BTreeSet<String>,
    oid: &str,
    expected_parent_oids: &[String],
    expected_tree_oid: &str,
) -> Result<(), ApiError> {
    let actual_tree_oid = git_object_field(source_repo, oid, "%T")?;
    if actual_tree_oid != expected_tree_oid {
        return Err(ApiError::internal_message(format!(
            "native public commit {oid} tree does not match persisted provenance"
        )));
    }
    let actual_parent_oids = git_object_field(source_repo, oid, "%P")?
        .split_whitespace()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if actual_parent_oids != expected_parent_oids {
        return Err(ApiError::internal_message(format!(
            "native public commit {oid} parents do not match persisted provenance"
        )));
    }
    for parent_oid in expected_parent_oids {
        if seen_oids.contains(parent_oid) {
            continue;
        }
        let parent_is_public =
            git_is_ancestor(target_repo, parent_oid, range_base_oid).map_err(|error| {
                ApiError::internal_message(format!(
                    "checking native parent {parent_oid} against public base {range_base_oid}: {}",
                    error.operator_diagnostic()
                ))
            })?;
        if parent_is_public {
            continue;
        }
        return Err(ApiError::internal_message(format!(
            "native public commit {oid} reaches a parent outside public history"
        )));
    }

    let mut revisions = format!("{oid}\n");
    for parent_oid in expected_parent_oids {
        revisions.push('^');
        revisions.push_str(parent_oid);
        revisions.push('\n');
    }
    let pack = git_process_output_with_timeout(
        Command::new("git").arg("--git-dir").arg(source_repo).args([
            "pack-objects",
            "--revs",
            "--stdout",
            "--thin",
        ]),
        Some(revisions.into_bytes()),
        RuntimeBudgets::default_git_command_timeout(),
    )?;
    if !pack.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "packing native public commit {oid}: {}",
            truncated_git_stderr(&pack.stderr)
        )));
    }
    let indexed = git_process_output_with_timeout(
        Command::new("git").arg("--git-dir").arg(target_repo).args([
            "index-pack",
            "--stdin",
            "--fix-thin",
        ]),
        Some(pack.stdout),
        RuntimeBudgets::default_git_command_timeout(),
    )?;
    if !indexed.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "indexing native public commit {oid}: {}",
            truncated_git_stderr(&indexed.stderr)
        )));
    }
    Ok(())
}

fn finish_native_range(
    repo_path: &FsPath,
    index_path: &FsPath,
    visible_tree: &BTreeMap<GitTreePath, ProjectionTreeFile>,
    range: NativeRangeState,
) -> Result<(), ApiError> {
    if !git_is_ancestor(repo_path, &range.base_oid, &range.head_oid)? {
        return Err(ApiError::internal_message(format!(
            "native public range {} does not descend from current public main",
            range.logical_commit_id
        )));
    }
    let expected_tree_oid = write_projection_tree(repo_path, index_path, visible_tree)?;
    let actual_tree_oid = git_object_field(repo_path, &range.head_oid, "%T")?;
    if actual_tree_oid != expected_tree_oid {
        return Err(ApiError::internal_message(format!(
            "native public range {} does not match the projected public tree",
            range.logical_commit_id
        )));
    }
    Ok(())
}

fn git_object_field(repo_path: &FsPath, oid: &str, format: &str) -> Result<String, ApiError> {
    let output = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo_path)
            .arg("show")
            .arg("--no-patch")
            .arg(format!("--format={format}"))
            .arg(oid),
        None,
    )?;
    String::from_utf8(output)
        .map_err(ApiError::bad_request)
        .map(|value| value.trim().to_string())
}

fn git_is_ancestor(
    repo_path: &FsPath,
    ancestor_oid: &str,
    descendant_oid: &str,
) -> Result<bool, ApiError> {
    let output = git_process_output_with_timeout(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo_path)
            .arg("merge-base")
            .arg("--is-ancestor")
            .arg(ancestor_oid)
            .arg(descendant_oid),
        None,
        RuntimeBudgets::default_git_command_timeout(),
    )?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    Err(ApiError::infrastructure_unavailable(truncated_git_stderr(
        &output.stderr,
    )))
}

pub(crate) fn projection_bare_repo_for_state(
    state: &AppState,
    repository_id: &str,
    projection: &Projection,
    git_head: Option<&scope_domain::repository::git::GitHead>,
    git_pack_spans: &[scope_domain::repository::git::GitPackSpan],
) -> Result<GitRepoHandle, ApiError> {
    let cache_root = state.repository_engine.cache_root().to_path_buf();
    let cache_key = projection_cache_key(projection);
    let repo_path = cache_root.join(format!("{cache_key}.git"));
    let is_ready = || projection_cache_is_ready(&repo_path);
    state.repository_engine.materialize_derived(
        repository_id,
        GitDerivedCacheNamespace::Projection,
        cache_key,
        &repo_path,
        is_ready,
        || {
            let raw_source_repo = if projection_requires_raw_source(projection) {
                let head = git_head.ok_or_else(|| {
                    ApiError::internal_message(
                        "Git-backed projection requires a canonical Git head",
                    )
                })?;
                Some(state.repository_engine.materialize_repository(
                    state,
                    repository_id,
                    head,
                    git_pack_spans,
                )?)
            } else {
                None
            };
            let _permit = state.runtime_budgets.acquire_git_materialization()?;
            projection_bare_repo_with_loader(
                &cache_root,
                projection,
                raw_source_repo.as_deref(),
                |blob| source_content_bytes_from_repo(state, blob, raw_source_repo.as_deref()),
            )
            .map(|_| ())
        },
    )
}

pub(crate) fn verify_projection_materialization(
    state: &AppState,
    projection: &Projection,
    native_source_repo: &FsPath,
    _git_manifest: &scope_domain::content::SourceBlob,
) -> Result<(), ApiError> {
    let cache_root = state.repository_engine.cache_root().to_path_buf();
    let attempt = PROJECTION_CACHE_ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let verification_root = cache_root.join(format!(
        ".projection-preflight.{}.{}",
        std::process::id(),
        attempt
    ));
    fs::create_dir_all(&verification_root).map_err(ApiError::internal)?;
    let verification = projection_bare_repo_with_loader(
        &verification_root,
        projection,
        Some(native_source_repo),
        |blob| source_content_bytes_from_repo(state, blob, Some(native_source_repo)),
    )
    .map(|_| ());
    let cleanup = fs::remove_dir_all(&verification_root).map_err(ApiError::internal);
    match (verification, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
    }
}

fn projection_requires_raw_source(projection: &Projection) -> bool {
    projection.commits.iter().any(|commit| {
        matches!(
            commit.materialization,
            ProjectionMaterialization::PreserveGitCommit { .. }
        ) || commit.changes.iter().any(|change| {
            change
                .new_content
                .as_ref()
                .is_some_and(|blob| matches!(blob.content_ref, ContentRef::GitBlob { .. }))
        })
    })
}

fn projection_cache_is_ready(repo_path: &FsPath) -> bool {
    repo_path
        .join("refs")
        .join("heads")
        .join(DEFAULT_GIT_BRANCH)
        .is_file()
}

fn projection_cache_key(projection: &Projection) -> String {
    let mut hasher = Sha1::new();
    hash_field(
        &mut hasher,
        b"semantics",
        PROJECTION_CACHE_SEMANTICS_VERSION.as_bytes(),
    );
    hash_field(&mut hasher, b"repo", projection.repo_id.as_bytes());
    hash_field(
        &mut hasher,
        b"view",
        projection.view_key.as_str().as_bytes(),
    );
    for commit in &projection.commits {
        hash_field(&mut hasher, b"commit", commit.projected_id.as_bytes());
        hash_field(&mut hasher, b"logical", commit.logical_commit_id.as_bytes());
        if let Some(parent) = &commit.parent_projected_id {
            hash_field(&mut hasher, b"parent", parent.as_bytes());
        }
        hash_field(&mut hasher, b"message", commit.message.as_bytes());
        match &commit.materialization {
            ProjectionMaterialization::Generate => {
                hash_field(&mut hasher, b"materialization", b"generate");
            }
            ProjectionMaterialization::PreserveGitCommit {
                oid,
                parent_oids,
                tree_oid,
            } => {
                hash_field(&mut hasher, b"materialization", b"preserve");
                hash_field(&mut hasher, b"native_oid", oid.as_bytes());
                hash_field(&mut hasher, b"native_tree", tree_oid.as_bytes());
                for parent_oid in parent_oids {
                    hash_field(&mut hasher, b"native_parent", parent_oid.as_bytes());
                }
            }
        }
        for change in &commit.changes {
            hash_field(&mut hasher, b"path", change.path.as_str().as_bytes());
            match &change.new_content {
                Some(blob) => {
                    hash_field(&mut hasher, b"sha256", blob.sha256.as_bytes());
                    hash_field(&mut hasher, b"git_oid", blob.git_oid.as_bytes());
                    hash_field(&mut hasher, b"mode", blob.git_file_mode.as_bytes());
                    hash_field(&mut hasher, b"size", blob.size_bytes.to_string().as_bytes());
                }
                None => hash_field(&mut hasher, b"delete", b""),
            }
        }
    }
    hex::encode(hasher.finalize())
}

pub(crate) fn hash_field(hasher: &mut Sha1, label: &[u8], value: &[u8]) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn write_projection_tree(
    repo_path: &FsPath,
    index_path: &FsPath,
    visible_tree: &BTreeMap<GitTreePath, ProjectionTreeFile>,
) -> Result<String, ApiError> {
    if index_path.exists() {
        fs::remove_file(index_path).map_err(ApiError::internal)?;
    }
    git_index_command(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo_path)
            .arg("read-tree")
            .arg("--empty"),
        index_path,
        None,
    )?;

    let mut index_info = Vec::new();
    for (path, file) in visible_tree {
        if !is_supported_git_file_mode(&file.git_file_mode) {
            return Err(ApiError::internal_message(format!(
                "projected Git path {path} has unsupported mode {}",
                file.git_file_mode
            )));
        }
        let oid = git_command_output(
            Command::new("git")
                .arg("--git-dir")
                .arg(repo_path)
                .arg("hash-object")
                .arg("-w")
                .arg("--stdin"),
            Some(&file.bytes),
        )?;
        let oid = String::from_utf8(oid).map_err(ApiError::bad_request)?;
        index_info.extend_from_slice(
            format!("{} blob {}\t{path}\0", file.git_file_mode, oid.trim()).as_bytes(),
        );
    }

    if !index_info.is_empty() {
        git_index_command(
            Command::new("git")
                .arg("--git-dir")
                .arg(repo_path)
                .arg("update-index")
                .arg("-z")
                .arg("--index-info"),
            index_path,
            Some(&index_info),
        )?;
    }
    let tree = git_index_command(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo_path)
            .arg("write-tree"),
        index_path,
        None,
    )?;
    let tree = String::from_utf8(tree).map_err(ApiError::bad_request)?;
    Ok(tree.trim().to_string())
}

#[derive(Clone)]
struct ProjectionTreeFile {
    bytes: Vec<u8>,
    git_file_mode: String,
}

fn git_commit_tree(
    repo_path: &FsPath,
    tree: &str,
    parent: Option<&str>,
    message: &str,
) -> Result<String, ApiError> {
    let mut command = Command::new("git");
    command
        .arg("--git-dir")
        .arg(repo_path)
        .arg("commit-tree")
        .arg(tree)
        .env("GIT_AUTHOR_NAME", "Scope")
        .env("GIT_AUTHOR_EMAIL", "scope@example.invalid")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_NAME", "Scope")
        .env("GIT_COMMITTER_EMAIL", "scope@example.invalid")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z");
    if let Some(parent) = parent {
        command.arg("-p").arg(parent.trim());
    }
    let output = git_command_output(&mut command, Some(message.as_bytes()))?;
    String::from_utf8(output).map_err(ApiError::bad_request)
}

fn git_index_command(
    command: &mut Command,
    index_path: &FsPath,
    stdin: Option<&[u8]>,
) -> Result<Vec<u8>, ApiError> {
    command.env("GIT_INDEX_FILE", index_path);
    git_command_output(command, stdin)
}

#[cfg(test)]
mod tests;
