use crate::error::ApiError;
use scope_domain::repository::git::{
    GitHead, GitPackSpan, GitSegmentUpload, GitSegmentUploadState,
};
use scope_object_store::ObjectStore;
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

pub(super) fn store_seed_git_pack(
    object_store: &dyn ObjectStore,
    git_segment_store: &scope_git_storage::GitSegmentStore,
    repository_id: &str,
    repo_path: &Path,
) -> Result<(GitHead, GitPackSpan, GitSegmentUpload), ApiError> {
    let head_oid = super::seed_git_head(repo_path)?;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["pack-objects", "--revs", "--stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ApiError::internal)?;
    child
        .stdin
        .take()
        .ok_or_else(|| ApiError::internal_message("seed pack stdin unavailable"))?
        .write_all(format!("{head_oid}\n").as_bytes())
        .map_err(ApiError::internal)?;
    let output = child.wait_with_output().map_err(ApiError::internal)?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "creating seeded Git pack: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let store = git_segment_store.clone();
    let repository_id = repository_id.to_string();
    let ingest_repository_id = repository_id.clone();
    let staged =
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(scope_git_storage::GitStorageError::Local)?
                .block_on(store.ingest_blocking_reader(
                    &ingest_repository_id,
                    std::io::Cursor::new(output.stdout),
                ))
        })
        .join()
        .map_err(|_| ApiError::internal_message("seed Git segment upload thread panicked"))?
        .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?;
    let stored = scope_git::prepare_git_push(
        staged.segment.clone(),
        head_oid,
        None,
        crate::config::default_git_storage_limits(),
    )?
    .store_manifest(object_store)?;
    let upload = GitSegmentUpload {
        segment_id: staged.segment.segment_id.clone(),
        repository_id,
        object_key: staged.object_key,
        state: GitSegmentUploadState::Published,
        sha256: Some(staged.segment.sha256.clone()),
        plaintext_bytes: Some(staged.segment.plaintext_bytes),
        encrypted_bytes: Some(staged.encrypted_bytes),
        encoding_version: staged.segment.encoding_version,
        created_at_unix: 1_800_000_000,
        updated_at_unix: 1_800_000_000,
    };
    Ok((stored.head, stored.pack_span, upload))
}

#[cfg(test)]
pub(crate) fn test_seed_git_segment_store() -> scope_git_storage::GitSegmentStore {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    static ATTEMPT: AtomicU64 = AtomicU64::new(1);
    let root = std::env::temp_dir().join(format!(
        "scope-seed-segments-{}-{}",
        std::process::id(),
        ATTEMPT.fetch_add(1, Ordering::Relaxed)
    ));
    scope_git_storage::GitSegmentStore::new(
        Arc::new(scope_git_storage::MemoryMultipartStore::default()),
        scope_git_storage::SegmentEncryptionKey::new("test", [3_u8; 32]).unwrap(),
        scope_git_storage::GitSegmentStoreConfig::new(root),
    )
    .unwrap()
}
