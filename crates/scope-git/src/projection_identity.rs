use crate::{GitTreePath, GitTreePathError};
use scope_domain::{
    content::is_supported_git_file_mode,
    projection::{Projection, ProjectionMaterialization},
};
use sha1::{Digest, Sha1};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};
use thiserror::Error;

pub const PROJECTION_IDENTITY_VERSION: i16 = 2;

const GENERATED_COMMIT_NAME: &str = "Scope";
const GENERATED_COMMIT_EMAIL: &str = "scope@example.invalid";
const GENERATED_COMMIT_TIME: &str = "946684800 +0000";

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ProjectionIdentityError {
    #[error(transparent)]
    InvalidPath(#[from] GitTreePathError),
    #[error("projected Git path {path} conflicts with another file or directory")]
    PathConflict { path: String },
    #[error("projected Git path {path} has unsupported mode {mode}")]
    UnsupportedMode { path: String, mode: String },
    #[error("projected Git object ID for {field} must be a 40-character hexadecimal SHA-1")]
    InvalidObjectId { field: &'static str },
    #[error("preserved Git commit identity does not match projected identity")]
    PreservedCommitIdentityMismatch,
    #[error("preserved Git commit tree does not match the projected tree")]
    PreservedCommitTreeMismatch,
    #[error("preserved Git history requires an existing generated projection base")]
    MissingPreservedCommitBase,
    #[error("preserved Git history does not descend from the current projected head")]
    PreservedHistoryNotDescendant,
}

/// Calculates the real Git commit identity produced by Scope's deterministic
/// projection materializer without reading blob bytes or invoking Git.
///
/// An empty domain projection has no canonical head. Filesystem adapters may
/// still create an implementation-only empty commit when serving a clone.
pub fn projection_head_oid(
    projection: &Projection,
) -> Result<Option<String>, ProjectionIdentityError> {
    if projection.commits.is_empty() {
        return Ok(None);
    }

    let mut tree = Tree::default();
    let mut parent_oid: Option<String> = None;
    let mut native_range: Option<NativeRange> = None;
    for commit in &projection.commits {
        if native_range
            .as_ref()
            .is_some_and(|range: &NativeRange| range.logical_commit_id != commit.logical_commit_id)
        {
            validate_native_range(
                native_range.take().expect("native range checked"),
                &tree.oid()?,
            )?;
        }

        let mut delta = BTreeMap::<GitTreePath, Option<TreeFile>>::new();
        for change in &commit.changes {
            let path = GitTreePath::from_scope_path(&change.path)?;
            match &change.new_content {
                Some(blob) => {
                    if !is_supported_git_file_mode(&blob.git_file_mode) {
                        return Err(ProjectionIdentityError::UnsupportedMode {
                            path: change.path.as_str().to_string(),
                            mode: blob.git_file_mode.clone(),
                        });
                    }
                    delta.insert(
                        path,
                        Some(TreeFile {
                            mode: blob.git_file_mode.clone(),
                            oid: parse_oid(&blob.git_oid, "blob")?,
                        }),
                    );
                }
                None => {
                    delta.insert(path, None);
                }
            }
        }
        for (path, file) in &delta {
            if file.is_none() {
                tree.remove(&path.as_str().split('/').collect::<Vec<_>>());
            }
        }
        for (path, file) in delta {
            if let Some(file) = file {
                tree.insert(&path.as_str().split('/').collect::<Vec<_>>(), file)?;
            }
        }

        let tree_oid = tree.oid()?;
        parent_oid = Some(match &commit.materialization {
            ProjectionMaterialization::Generate => {
                if let Some(range) = native_range.take() {
                    validate_native_range(range, &tree_oid)?;
                }
                generated_commit_oid(
                    &tree_oid,
                    parent_oid.as_deref(),
                    &format!("{}\n", commit.message),
                )
            }
            ProjectionMaterialization::PreserveGitCommit {
                oid,
                parent_oids,
                tree_oid: expected_tree_oid,
            } => {
                if oid != &commit.projected_id {
                    return Err(ProjectionIdentityError::PreservedCommitIdentityMismatch);
                }
                let oid = parse_oid(oid, "preserved commit")?;
                let expected_tree_oid = parse_oid(expected_tree_oid, "preserved tree")?;
                let oid = hex::encode(oid);
                let base_oid = parent_oid
                    .as_ref()
                    .ok_or(ProjectionIdentityError::MissingPreservedCommitBase)?;
                native_range.get_or_insert_with(|| NativeRange {
                    logical_commit_id: commit.logical_commit_id.clone(),
                    base_oid: base_oid.clone(),
                    expected_tree_oid,
                    descendants_of_base: BTreeSet::new(),
                    head_descends_from_base: false,
                });
                let range = native_range.as_mut().expect("native range initialized");
                let descends_from_base = parent_oids.iter().any(|parent| {
                    parent == &range.base_oid || range.descendants_of_base.contains(parent)
                });
                if descends_from_base {
                    range.descendants_of_base.insert(oid.clone());
                }
                range.expected_tree_oid = expected_tree_oid;
                range.head_descends_from_base = descends_from_base;
                oid
            }
        });
    }

    if let Some(range) = native_range {
        validate_native_range(range, &tree.oid()?)?;
    }

    Ok(parent_oid)
}

struct NativeRange {
    logical_commit_id: String,
    base_oid: String,
    expected_tree_oid: [u8; 20],
    descendants_of_base: BTreeSet<String>,
    head_descends_from_base: bool,
}

fn validate_native_range(
    range: NativeRange,
    projected_tree_oid: &[u8; 20],
) -> Result<(), ProjectionIdentityError> {
    if &range.expected_tree_oid != projected_tree_oid {
        return Err(ProjectionIdentityError::PreservedCommitTreeMismatch);
    }
    if range.head_descends_from_base {
        Ok(())
    } else {
        Err(ProjectionIdentityError::PreservedHistoryNotDescendant)
    }
}

fn parse_oid(value: &str, field: &'static str) -> Result<[u8; 20], ProjectionIdentityError> {
    let bytes = hex::decode(value)
        .ok()
        .filter(|bytes| bytes.len() == 20)
        .ok_or(ProjectionIdentityError::InvalidObjectId { field })?;
    let mut oid = [0_u8; 20];
    oid.copy_from_slice(&bytes);
    Ok(oid)
}

fn generated_commit_oid(tree_oid: &[u8; 20], parent_oid: Option<&str>, message: &str) -> String {
    let tree_oid = hex::encode(tree_oid);
    let mut payload = format!("tree {tree_oid}\n");
    if let Some(parent_oid) = parent_oid {
        payload.push_str(&format!("parent {parent_oid}\n"));
    }
    payload.push_str(&format!(
        "author {GENERATED_COMMIT_NAME} <{GENERATED_COMMIT_EMAIL}> {GENERATED_COMMIT_TIME}\n\
         committer {GENERATED_COMMIT_NAME} <{GENERATED_COMMIT_EMAIL}> {GENERATED_COMMIT_TIME}\n\n\
         {message}"
    ));
    object_oid("commit", payload.as_bytes())
}

fn object_oid(kind: &str, payload: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("{kind} {}\0", payload.len()).as_bytes());
    hasher.update(payload);
    hex::encode(hasher.finalize())
}

#[derive(Clone)]
struct TreeFile {
    mode: String,
    oid: [u8; 20],
}

#[derive(Default)]
struct Tree {
    entries: BTreeMap<String, TreeEntry>,
    cached_oid: Option<[u8; 20]>,
}

enum TreeEntry {
    File(TreeFile),
    Directory(Tree),
}

impl Tree {
    fn insert(&mut self, path: &[&str], file: TreeFile) -> Result<(), ProjectionIdentityError> {
        let (name, rest) = path.split_first().expect("projection path is non-empty");
        if rest.is_empty() {
            if matches!(self.entries.get(*name), Some(TreeEntry::Directory(_))) {
                return Err(ProjectionIdentityError::PathConflict {
                    path: path.join("/"),
                });
            }
            self.entries
                .insert((*name).to_string(), TreeEntry::File(file));
            self.cached_oid = None;
            return Ok(());
        }

        let entry = self
            .entries
            .entry((*name).to_string())
            .or_insert_with(|| TreeEntry::Directory(Tree::default()));
        match entry {
            TreeEntry::Directory(directory) => {
                directory.insert(rest, file)?;
                self.cached_oid = None;
                Ok(())
            }
            TreeEntry::File(_) => Err(ProjectionIdentityError::PathConflict {
                path: path.join("/"),
            }),
        }
    }

    fn remove(&mut self, path: &[&str]) -> bool {
        let Some((name, rest)) = path.split_first() else {
            return false;
        };
        let changed = if rest.is_empty() {
            if matches!(self.entries.get(*name), Some(TreeEntry::File(_))) {
                self.entries.remove(*name);
                true
            } else {
                false
            }
        } else {
            let (changed, empty) = match self.entries.get_mut(*name) {
                Some(TreeEntry::Directory(directory)) => {
                    let changed = directory.remove(rest);
                    (changed, directory.entries.is_empty())
                }
                Some(TreeEntry::File(_)) | None => (false, false),
            };
            if empty {
                self.entries.remove(*name);
            }
            changed
        };
        if changed {
            self.cached_oid = None;
        }
        changed
    }

    fn oid(&mut self) -> Result<[u8; 20], ProjectionIdentityError> {
        if let Some(oid) = self.cached_oid {
            return Ok(oid);
        }
        let mut entries = self
            .entries
            .iter()
            .map(|(name, entry)| (name.clone(), entry.is_directory()))
            .collect::<Vec<_>>();
        entries.sort_by(
            |(left_name, left_directory), (right_name, right_directory)| {
                git_tree_name_cmp(left_name, *left_directory, right_name, *right_directory)
            },
        );

        let mut payload = Vec::new();
        for (name, _) in entries {
            let entry = self.entries.get_mut(&name).expect("tree entry was listed");
            let (mode, oid) = match entry {
                TreeEntry::File(file) => (file.mode.as_str(), file.oid),
                TreeEntry::Directory(directory) => ("40000", directory.oid()?),
            };
            payload.extend_from_slice(mode.as_bytes());
            payload.push(b' ');
            payload.extend_from_slice(name.as_bytes());
            payload.push(0);
            payload.extend_from_slice(&oid);
        }
        let oid = parse_oid(&object_oid("tree", &payload), "tree")?;
        self.cached_oid = Some(oid);
        Ok(oid)
    }
}

impl TreeEntry {
    fn is_directory(&self) -> bool {
        matches!(self, Self::Directory(_))
    }
}

fn git_tree_name_cmp(
    left: &str,
    left_directory: bool,
    right: &str,
    right_directory: bool,
) -> Ordering {
    let left = left
        .as_bytes()
        .iter()
        .copied()
        .chain(left_directory.then_some(b'/'));
    let right = right
        .as_bytes()
        .iter()
        .copied()
        .chain(right_directory.then_some(b'/'));
    left.cmp(right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::{
        content::SourceBlob,
        content_ref::ContentRef,
        policy::{ScopePath, Visibility},
        projection::{ProjectedChange, ProjectedCommit, ProjectionViewKey},
    };
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Command, Stdio},
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn generated_projection_identity_matches_git_for_nested_modes_and_deletion() {
        let readme = blob(b"readme", "100644");
        let script = blob(b"#!/bin/sh\necho scope\n", "100755");
        let replacement = blob(b"replacement", "100644");
        let projection = Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits: vec![
                generated_commit(
                    "one",
                    None,
                    "Initial projection",
                    vec![
                        change("/README.md", Some(readme.clone())),
                        change("/tools/run", Some(script.clone())),
                    ],
                ),
                generated_commit(
                    "two",
                    Some("one"),
                    "Update projection",
                    vec![
                        change("/README.md", None),
                        change("/docs/guide.txt", Some(replacement.clone())),
                    ],
                ),
            ],
        };

        let expected = materialize_with_git(
            &projection,
            &[
                (&readme, b"readme"),
                (&script, b"#!/bin/sh\necho scope\n"),
                (&replacement, b"replacement"),
            ],
        );
        assert_eq!(projection_head_oid(&projection).unwrap(), Some(expected));
    }

    #[test]
    fn preserved_and_mixed_projection_identity_matches_git() {
        let base_blob = blob(b"base", "100644");
        let native_blob = blob(b"native", "100644");
        let final_blob = blob(b"final", "100755");
        let base = generated_commit(
            "base",
            None,
            "Base projection",
            vec![change("/file", Some(base_blob.clone()))],
        );
        let native_as_generated = generated_commit(
            "native-generated",
            Some("base"),
            "Native projection",
            vec![change("/file", Some(native_blob.clone()))],
        );
        let generated_native_projection = Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits: vec![base.clone(), native_as_generated],
        };
        let base_oid = projection_head_oid(&Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits: vec![base.clone()],
        })
        .unwrap()
        .unwrap();
        let native_oid = projection_head_oid(&generated_native_projection)
            .unwrap()
            .unwrap();
        let native_tree_oid = tree_oid(&[("file", &native_blob)]);
        let native = ProjectedCommit {
            projected_id: native_oid.clone(),
            logical_commit_id: "native".to_string(),
            parent_projected_id: Some("base".to_string()),
            author: None,
            message: "Native projection".to_string(),
            changes: vec![change("/file", Some(native_blob.clone()))],
            materialization: ProjectionMaterialization::PreserveGitCommit {
                oid: native_oid.clone(),
                parent_oids: vec![base_oid],
                tree_oid: native_tree_oid,
            },
        };
        let preserved = Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits: vec![base.clone(), native.clone()],
        };
        let blobs = [
            (&base_blob, b"base".as_slice()),
            (&native_blob, b"native".as_slice()),
            (&final_blob, b"final".as_slice()),
        ];

        assert_eq!(
            projection_head_oid(&preserved).unwrap(),
            Some(materialize_with_git(&preserved, &blobs))
        );

        let mixed = Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits: vec![
                base,
                native,
                generated_commit(
                    "final",
                    Some("native"),
                    "Final projection",
                    vec![change("/bin/run", Some(final_blob.clone()))],
                ),
            ],
        };
        assert_eq!(
            projection_head_oid(&mixed).unwrap(),
            Some(materialize_with_git(&mixed, &blobs))
        );
    }

    #[test]
    fn file_directory_transitions_validate_the_complete_commit_delta() {
        let nested = blob(b"nested", "100644");
        let flat = blob(b"flat", "100644");
        let replacement = blob(b"replacement", "100644");
        let projection = Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits: vec![
                generated_commit(
                    "base",
                    None,
                    "Base",
                    vec![change("/a/file", Some(nested.clone()))],
                ),
                generated_commit(
                    "to-file",
                    Some("base"),
                    "Directory to file",
                    vec![change("/a", Some(flat.clone())), change("/a/file", None)],
                ),
                generated_commit(
                    "to-directory",
                    Some("to-file"),
                    "File to directory",
                    vec![
                        change("/a/file", Some(replacement.clone())),
                        change("/a", None),
                    ],
                ),
            ],
        };
        let blobs = [
            (&nested, b"nested".as_slice()),
            (&flat, b"flat".as_slice()),
            (&replacement, b"replacement".as_slice()),
        ];

        assert_eq!(
            projection_head_oid(&projection).unwrap(),
            Some(materialize_with_git(&projection, &blobs))
        );
    }

    #[test]
    fn preserved_merge_allows_a_side_parent_outside_the_projection_sequence() {
        let content = blob(b"content", "100644");
        let base = generated_commit(
            "base",
            None,
            "Base",
            vec![change("/file", Some(content.clone()))],
        );
        let base_oid = projection_head_oid(&Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits: vec![base.clone()],
        })
        .unwrap()
        .unwrap();
        let merge_oid = "1111111111111111111111111111111111111111";
        let projection = Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits: vec![
                base,
                ProjectedCommit {
                    projected_id: merge_oid.to_string(),
                    logical_commit_id: "native-merge".to_string(),
                    parent_projected_id: Some(base_oid.clone()),
                    author: None,
                    message: "Native merge".to_string(),
                    changes: Vec::new(),
                    materialization: ProjectionMaterialization::PreserveGitCommit {
                        oid: merge_oid.to_string(),
                        parent_oids: vec![
                            base_oid,
                            "2222222222222222222222222222222222222222".to_string(),
                        ],
                        tree_oid: tree_oid(&[("file", &content)]),
                    },
                },
            ],
        };

        assert_eq!(
            projection_head_oid(&projection).unwrap(),
            Some(merge_oid.to_string())
        );
    }

    #[test]
    fn long_generated_history_uses_linear_identity_state() {
        let mut commits = Vec::with_capacity(10_000);
        commits.push(generated_commit(
            "commit-0",
            None,
            "Commit 0",
            vec![change("/file", Some(blob(b"content", "100644")))],
        ));
        for index in 1..10_000 {
            commits.push(generated_commit(
                &format!("commit-{index}"),
                Some(&format!("commit-{}", index - 1)),
                &format!("Commit {index}"),
                Vec::new(),
            ));
        }
        let projection = Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits,
        };

        assert_eq!(projection_head_oid(&projection).unwrap().unwrap().len(), 40);
    }

    #[test]
    fn empty_projection_has_no_canonical_head() {
        let projection = Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits: Vec::new(),
        };

        assert_eq!(projection_head_oid(&projection).unwrap(), None);
    }

    #[test]
    fn preserved_commit_must_match_the_projected_tree() {
        let content = blob(b"content", "100644");
        let base = generated_commit("base", None, "Base", vec![change("/file", Some(content))]);
        let base_oid = projection_head_oid(&Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits: vec![base.clone()],
        })
        .unwrap()
        .unwrap();
        let projection = Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits: vec![
                base,
                ProjectedCommit {
                    projected_id: "1111111111111111111111111111111111111111".to_string(),
                    logical_commit_id: "native".to_string(),
                    parent_projected_id: Some("base".to_string()),
                    author: None,
                    message: "Native".to_string(),
                    changes: Vec::new(),
                    materialization: ProjectionMaterialization::PreserveGitCommit {
                        oid: "1111111111111111111111111111111111111111".to_string(),
                        parent_oids: vec![base_oid],
                        tree_oid: "2222222222222222222222222222222222222222".to_string(),
                    },
                },
            ],
        };

        assert_eq!(
            projection_head_oid(&projection).unwrap_err(),
            ProjectionIdentityError::PreservedCommitTreeMismatch
        );
    }

    #[test]
    fn preserved_history_cannot_skip_the_current_projected_head() {
        let content = blob(b"content", "100644");
        let first = generated_commit(
            "first",
            None,
            "First",
            vec![change("/file", Some(content.clone()))],
        );
        let second = generated_commit("second", Some("first"), "Second", Vec::new());
        let first_oid = projection_head_oid(&Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits: vec![first.clone()],
        })
        .unwrap()
        .unwrap();
        let skipped_head = "3333333333333333333333333333333333333333";
        let projection = Projection {
            repo_id: "owner/repo".to_string(),
            view_key: ProjectionViewKey::Public,
            commits: vec![
                first,
                second,
                ProjectedCommit {
                    projected_id: skipped_head.to_string(),
                    logical_commit_id: "native".to_string(),
                    parent_projected_id: Some("second".to_string()),
                    author: None,
                    message: "Native".to_string(),
                    changes: Vec::new(),
                    materialization: ProjectionMaterialization::PreserveGitCommit {
                        oid: skipped_head.to_string(),
                        parent_oids: vec![first_oid],
                        tree_oid: tree_oid(&[("file", &content)]),
                    },
                },
            ],
        };

        assert_eq!(
            projection_head_oid(&projection).unwrap_err(),
            ProjectionIdentityError::PreservedHistoryNotDescendant
        );
    }

    fn blob(bytes: &[u8], mode: &str) -> SourceBlob {
        let oid = object_oid("blob", bytes);
        SourceBlob {
            content_ref: ContentRef::blob_sha256(format!("sha-{oid}")),
            sha256: format!("sha-{oid}"),
            git_oid: oid,
            git_file_mode: mode.to_string(),
            size_bytes: bytes.len() as u64,
        }
    }

    fn change(path: &str, new_content: Option<SourceBlob>) -> ProjectedChange {
        ProjectedChange {
            path: ScopePath::parse(path).unwrap(),
            new_content,
            visibility: Visibility::Public,
        }
    }

    fn generated_commit(
        id: &str,
        parent: Option<&str>,
        message: &str,
        changes: Vec<ProjectedChange>,
    ) -> ProjectedCommit {
        ProjectedCommit {
            projected_id: id.to_string(),
            logical_commit_id: id.to_string(),
            parent_projected_id: parent.map(str::to_string),
            author: None,
            message: message.to_string(),
            changes,
            materialization: ProjectionMaterialization::Generate,
        }
    }

    fn tree_oid(files: &[(&str, &SourceBlob)]) -> String {
        let mut tree = Tree::default();
        for (path, blob) in files {
            tree.insert(
                &path.split('/').collect::<Vec<_>>(),
                TreeFile {
                    mode: blob.git_file_mode.clone(),
                    oid: parse_oid(&blob.git_oid, "test blob").unwrap(),
                },
            )
            .unwrap();
        }
        hex::encode(tree.oid().unwrap())
    }

    fn materialize_with_git(projection: &Projection, blobs: &[(&SourceBlob, &[u8])]) -> String {
        let root = fixture_root();
        let repo = root.join("repo.git");
        let index = root.join("index");
        run(
            Command::new("git").arg("init").arg("--bare").arg(&repo),
            None,
        );

        let content_by_oid = blobs
            .iter()
            .map(|(blob, bytes)| (blob.git_oid.clone(), *bytes))
            .collect::<BTreeMap<_, _>>();
        for (oid, bytes) in &content_by_oid {
            let actual = output(
                Command::new("git")
                    .arg("--git-dir")
                    .arg(&repo)
                    .arg("hash-object")
                    .arg("-w")
                    .arg("--stdin"),
                Some(bytes),
                None,
            );
            assert_eq!(actual.trim(), oid);
        }

        let mut visible = BTreeMap::<String, SourceBlob>::new();
        let mut parent: Option<String> = None;
        for commit in &projection.commits {
            for change in &commit.changes {
                let path = change.path.as_str().trim_start_matches('/').to_string();
                match &change.new_content {
                    Some(blob) => {
                        visible.insert(path, blob.clone());
                    }
                    None => {
                        visible.remove(&path);
                    }
                }
            }
            run(
                Command::new("git")
                    .arg("--git-dir")
                    .arg(&repo)
                    .arg("read-tree")
                    .arg("--empty"),
                Some(&index),
            );
            for (path, blob) in &visible {
                run(
                    Command::new("git")
                        .arg("--git-dir")
                        .arg(&repo)
                        .arg("update-index")
                        .arg("--add")
                        .arg("--cacheinfo")
                        .arg(format!("{},{},{}", blob.git_file_mode, blob.git_oid, path)),
                    Some(&index),
                );
            }
            let tree = output(
                Command::new("git")
                    .arg("--git-dir")
                    .arg(&repo)
                    .arg("write-tree"),
                None,
                Some(&index),
            );
            let mut command = Command::new("git");
            command
                .arg("--git-dir")
                .arg(&repo)
                .arg("commit-tree")
                .arg(tree.trim())
                .env("GIT_AUTHOR_NAME", GENERATED_COMMIT_NAME)
                .env("GIT_AUTHOR_EMAIL", GENERATED_COMMIT_EMAIL)
                .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
                .env("GIT_COMMITTER_NAME", GENERATED_COMMIT_NAME)
                .env("GIT_COMMITTER_EMAIL", GENERATED_COMMIT_EMAIL)
                .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z");
            match &commit.materialization {
                ProjectionMaterialization::Generate => {
                    if let Some(parent) = &parent {
                        command.arg("-p").arg(parent.trim());
                    }
                }
                ProjectionMaterialization::PreserveGitCommit {
                    oid,
                    parent_oids,
                    tree_oid,
                } => {
                    assert_eq!(tree.trim(), tree_oid);
                    for parent_oid in parent_oids {
                        command.arg("-p").arg(parent_oid);
                    }
                    let actual = output(
                        &mut command,
                        Some(format!("{}\n", commit.message).as_bytes()),
                        None,
                    );
                    assert_eq!(actual.trim(), oid);
                    parent = Some(actual);
                    continue;
                }
            }
            parent = Some(output(
                &mut command,
                Some(format!("{}\n", commit.message).as_bytes()),
                None,
            ));
        }
        let head = parent.unwrap().trim().to_string();
        fs::remove_dir_all(root).unwrap();
        head
    }

    fn fixture_root() -> PathBuf {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scope-git-projection-identity-{}-{sequence}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn run(command: &mut Command, index: Option<&Path>) {
        let output = command_output(command, None, index);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn output(command: &mut Command, input: Option<&[u8]>, index: Option<&Path>) -> String {
        let output = command_output(command, input, index);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn command_output(
        command: &mut Command,
        input: Option<&[u8]>,
        index: Option<&Path>,
    ) -> std::process::Output {
        if let Some(index) = index {
            command.env("GIT_INDEX_FILE", index);
        }
        if input.is_some() {
            command.stdin(Stdio::piped());
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        if let Some(input) = input {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(input).unwrap();
        }
        child.wait_with_output().unwrap()
    }
}
