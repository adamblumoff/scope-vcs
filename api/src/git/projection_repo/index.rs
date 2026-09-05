use super::{git_command_output, git_index_command};
use crate::error::ApiError;
use scope_domain::{
    content::{SourceBlob, is_supported_git_file_mode},
    projection::ProjectedChange,
};
use scope_git::GitTreePath;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
};

/// Retains Git's index between commits. Only changed blobs are loaded and
/// written; unchanged entries and their object identities remain in the index.
pub(super) struct ProjectionIndex {
    repo: PathBuf,
    path: PathBuf,
    written_blobs: BTreeSet<(String, String)>,
    pub(super) loaded_bytes: u64,
    pub(super) written_blobs_count: usize,
}

impl ProjectionIndex {
    pub(super) fn new(repo: &Path, path: &Path, base: Option<&str>) -> Result<Self, ApiError> {
        let mut command = Command::new("git");
        command
            .arg("--git-dir")
            .arg(repo)
            .arg("read-tree")
            .arg(base.unwrap_or("--empty"));
        git_index_command(&mut command, path, None)?;
        Ok(Self {
            repo: repo.to_path_buf(),
            path: path.to_path_buf(),
            written_blobs: BTreeSet::new(),
            loaded_bytes: 0,
            written_blobs_count: 0,
        })
    }

    pub(super) fn remember_verified_blobs<'a>(
        &mut self,
        blobs: impl Iterator<Item = &'a SourceBlob>,
    ) {
        self.written_blobs
            .extend(blobs.map(|blob| (blob.git_oid.clone(), blob.sha256.clone())));
    }

    pub(super) fn apply(
        &mut self,
        changes: &[ProjectedChange],
        load: &impl Fn(&SourceBlob) -> Result<Vec<u8>, ApiError>,
    ) -> Result<(), ApiError> {
        let mut delta = BTreeMap::new();
        for change in changes {
            let path = GitTreePath::from_scope_path(&change.path).map_err(ApiError::internal)?;
            delta.insert(path, change.new_content.as_ref());
        }
        let mut removals = Vec::new();
        let mut additions = Vec::new();
        for (path, blob) in delta {
            let Some(blob) = blob else {
                removals.extend_from_slice(format!("0 {}\t{path}\0", "0".repeat(40)).as_bytes());
                continue;
            };
            if !is_supported_git_file_mode(&blob.git_file_mode) {
                return Err(ApiError::internal_message(format!(
                    "projected Git path {path} has unsupported mode {}",
                    blob.git_file_mode
                )));
            }
            if self
                .written_blobs
                .insert((blob.git_oid.clone(), blob.sha256.clone()))
            {
                let bytes = load(blob)?;
                let oid = git_command_output(
                    Command::new("git").arg("--git-dir").arg(&self.repo).args([
                        "hash-object",
                        "-w",
                        "--stdin",
                    ]),
                    Some(&bytes),
                )?;
                if std::str::from_utf8(&oid)
                    .map_err(ApiError::internal)?
                    .trim()
                    != blob.git_oid
                {
                    return Err(ApiError::internal_message(
                        "projected blob bytes do not match their Git identity",
                    ));
                }
                self.loaded_bytes += bytes.len() as u64;
                self.written_blobs_count += 1;
            }
            additions.extend_from_slice(
                format!("{} blob {}\t{path}\0", blob.git_file_mode, blob.git_oid).as_bytes(),
            );
        }
        removals.extend_from_slice(&additions);
        if !removals.is_empty() {
            git_index_command(
                Command::new("git").arg("--git-dir").arg(&self.repo).args([
                    "update-index",
                    "-z",
                    "--index-info",
                ]),
                &self.path,
                Some(&removals),
            )?;
        }
        Ok(())
    }

    pub(super) fn tree(&self) -> Result<String, ApiError> {
        let bytes = git_index_command(
            Command::new("git")
                .arg("--git-dir")
                .arg(&self.repo)
                .arg("write-tree"),
            &self.path,
            None,
        )?;
        Ok(String::from_utf8(bytes)
            .map_err(ApiError::internal)?
            .trim()
            .to_string())
    }
}
