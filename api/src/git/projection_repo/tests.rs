use super::*;
use scope_domain::{
    content::SourceBlob,
    content_ref::ContentRef,
    policy::{ScopePath, Visibility},
    projection::{ProjectedChange, ProjectedCommit, ProjectionViewKey},
};
use scope_git::GitTreePath;
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
struct ProjectionTreeFile {
    bytes: Vec<u8>,
    git_file_mode: String,
}
fn write_projection_tree(
    repo: &FsPath,
    index_path: &FsPath,
    files: &BTreeMap<GitTreePath, ProjectionTreeFile>,
) -> Result<String, ApiError> {
    let changes = files
        .iter()
        .map(|(path, file)| {
            let mut change = projected_change(
                &format!("/{path}"),
                std::str::from_utf8(&file.bytes).unwrap(),
            );
            change
                .new_content
                .as_mut()
                .unwrap()
                .git_file_mode
                .clone_from(&file.git_file_mode);
            change
        })
        .collect::<Vec<_>>();
    let mut index = ProjectionIndex::new(repo, index_path, None)?;
    index.apply(&changes, &|blob| Ok(blob.sha256.as_bytes().to_vec()))?;
    index.tree()
}

#[test]
fn generated_projection_matches_canonical_head_identity() {
    let root = std::env::temp_dir().join(format!(
        "scope-generated-projection-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let projection = Projection {
        repo_id: "repo".to_string(),
        view_key: ProjectionViewKey::Public,
        commits: vec![ProjectedCommit {
            projected_id: "generated-base".to_string(),
            logical_commit_id: "logical-base".to_string(),
            parent_projected_id: None,
            author: Some("owner".to_string()),
            message: "base".to_string(),
            changes: vec![projected_change("/README.md", "base\n")],
            materialization: ProjectionMaterialization::Generate,
        }],
    };

    let repo = projection_bare_repo_with_loader(&root, None, &projection, None, None, |blob| {
        Ok(blob.sha256.as_bytes().to_vec())
    })
    .unwrap();

    assert_eq!(
        git_object_field(&repo, "refs/heads/main", "%H").unwrap(),
        scope_git::projection_head_oid(&projection)
            .unwrap()
            .expect("non-empty projection has a head")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn generated_projection_preserves_a_leading_quote_in_a_file_name() {
    let root = std::env::temp_dir().join(format!(
        "scope-quoted-projection-path-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let projection = Projection {
        repo_id: "repo".to_string(),
        view_key: ProjectionViewKey::Public,
        commits: vec![ProjectedCommit {
            projected_id: "generated-base".to_string(),
            logical_commit_id: "logical-base".to_string(),
            parent_projected_id: None,
            author: Some("owner".to_string()),
            message: "base".to_string(),
            changes: vec![projected_change("/\"quoted.txt", "quoted\n")],
            materialization: ProjectionMaterialization::Generate,
        }],
    };

    let repo = projection_bare_repo_with_loader(&root, None, &projection, None, None, |blob| {
        Ok(blob.sha256.as_bytes().to_vec())
    })
    .unwrap();
    let paths = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&repo)
            .arg("ls-tree")
            .arg("-r")
            .arg("--name-only")
            .arg("-z")
            .arg("refs/heads/main"),
        None,
    )
    .unwrap();

    assert_eq!(paths, b"\"quoted.txt\0");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn projection_identity_and_materializer_reject_the_same_reserved_path() {
    let root = std::env::temp_dir().join(format!(
        "scope-invalid-projection-path-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let projection = Projection {
        repo_id: "repo".to_string(),
        view_key: ProjectionViewKey::Public,
        commits: vec![ProjectedCommit {
            projected_id: "generated-base".to_string(),
            logical_commit_id: "logical-base".to_string(),
            parent_projected_id: None,
            author: Some("owner".to_string()),
            message: "base".to_string(),
            changes: vec![projected_change("/vendor/.GiT/config", "malicious\n")],
            materialization: ProjectionMaterialization::Generate,
        }],
    };

    let identity_error = scope_git::projection_head_oid(&projection)
        .unwrap_err()
        .to_string();
    let materialization_error =
        projection_bare_repo_with_loader(&root, None, &projection, None, None, |_| {
            panic!("invalid paths must fail before loading content")
        })
        .unwrap_err();

    assert_eq!(materialization_error.operator_diagnostic(), identity_error);
    assert_eq!(
        materialization_error.public_message(),
        "Scope hit an internal error."
    );
    assert!(identity_error.contains("reserved .git component"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_commit_is_reused_exactly_and_tree_corruption_fails_closed() {
    let root = std::env::temp_dir().join(format!(
        "scope-native-projection-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let source = root.join("source.git");
    let cache = root.join("cache");
    fs::create_dir_all(&cache).unwrap();
    git_command_output(
        Command::new("git").arg("init").arg("--bare").arg(&source),
        None,
    )
    .unwrap();

    let source_index = root.join("source.index");
    let source_tree = BTreeMap::from([(
        projection_tree_path("/README.md"),
        ProjectionTreeFile {
            bytes: b"base\n".to_vec(),
            git_file_mode: "100644".to_string(),
        },
    )]);
    let base_tree = write_projection_tree(&source, &source_index, &source_tree).unwrap();
    let base_oid = git_commit_tree(&source, &base_tree, None, "base\n")
        .unwrap()
        .trim()
        .to_string();
    let mut native_files = source_tree.clone();
    native_files.insert(
        projection_tree_path("/request.txt"),
        ProjectionTreeFile {
            bytes: b"contributor\n".to_vec(),
            git_file_mode: "100644".to_string(),
        },
    );
    let native_tree = write_projection_tree(&source, &source_index, &native_files).unwrap();
    let native_oid = git_commit_tree(&source, &native_tree, Some(&base_oid), "contributor\n")
        .unwrap()
        .trim()
        .to_string();
    let mut private_files = source_tree.clone();
    private_files.insert(
        projection_tree_path("/secret.txt"),
        ProjectionTreeFile {
            bytes: b"private\n".to_vec(),
            git_file_mode: "100644".to_string(),
        },
    );
    let private_tree = write_projection_tree(&source, &source_index, &private_files).unwrap();
    let private_oid = git_commit_tree(&source, &private_tree, Some(&base_oid), "private\n")
        .unwrap()
        .trim()
        .to_string();
    let secret_blob_oid = String::from_utf8(
        git_command_output(
            Command::new("git")
                .arg("--git-dir")
                .arg(&source)
                .arg("rev-parse")
                .arg(format!("{private_tree}:secret.txt")),
            None,
        )
        .unwrap(),
    )
    .unwrap()
    .trim()
    .to_string();
    let mut canonical_merge_files = private_files;
    canonical_merge_files.insert(
        projection_tree_path("/request.txt"),
        ProjectionTreeFile {
            bytes: b"contributor\n".to_vec(),
            git_file_mode: "100644".to_string(),
        },
    );
    let canonical_merge_tree =
        write_projection_tree(&source, &source_index, &canonical_merge_files).unwrap();
    let canonical_merge_oid = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&source)
            .arg("commit-tree")
            .arg(&canonical_merge_tree)
            .arg("-p")
            .arg(&private_oid)
            .arg("-p")
            .arg(&native_oid)
            .env("GIT_AUTHOR_NAME", "Scope")
            .env("GIT_AUTHOR_EMAIL", "scope@scope.local")
            .env("GIT_COMMITTER_NAME", "Scope")
            .env("GIT_COMMITTER_EMAIL", "scope@scope.local")
            .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z"),
        Some(b"merge\n"),
    )
    .unwrap();
    let canonical_merge_oid = String::from_utf8(canonical_merge_oid)
        .unwrap()
        .trim()
        .to_string();
    git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&source)
            .arg("update-ref")
            .arg(format!("refs/heads/{DEFAULT_GIT_BRANCH}"))
            .arg(&canonical_merge_oid),
        None,
    )
    .unwrap();

    let generated = ProjectedCommit {
        projected_id: "generated-base".to_string(),
        logical_commit_id: "logical-base".to_string(),
        parent_projected_id: None,
        author: Some("owner".to_string()),
        message: "base".to_string(),
        changes: vec![projected_change("/README.md", "base\n")],
        materialization: ProjectionMaterialization::Generate,
    };
    let preserved = ProjectedCommit {
        projected_id: native_oid.clone(),
        logical_commit_id: "logical-request".to_string(),
        parent_projected_id: Some(base_oid.clone()),
        author: None,
        message: "contributor".to_string(),
        changes: vec![projected_change("/request.txt", "contributor\n")],
        materialization: ProjectionMaterialization::PreserveGitCommit {
            oid: native_oid.clone(),
            parent_oids: vec![base_oid],
            tree_oid: native_tree,
        },
    };
    let projection = Projection {
        repo_id: "repo".to_string(),
        view_key: ProjectionViewKey::Public,
        commits: vec![generated, preserved],
    };

    let engine =
        crate::git::repository_engine::RepositoryEngine::new(cache.clone(), 100 * 1024 * 1024)
            .unwrap();
    let incarnation = RepositoryIncarnation::new("repo", "repoi_native").unwrap();
    let repo = projection_bare_repo_with_loader(
        &cache,
        Some(&incarnation),
        &projection,
        Some(&source),
        None,
        |blob| Ok(blob.sha256.as_bytes().to_vec()),
    )
    .unwrap();
    assert_eq!(
        git_object_field(&repo, "refs/heads/main", "%H").unwrap(),
        native_oid
    );
    assert!(!git_object_exists(&repo, &private_oid));
    assert!(!git_object_exists(&repo, &secret_blob_oid));
    assert!(!git_object_exists(&repo, &canonical_merge_oid));

    let mut extended = projection.clone();
    extended.commits.push(ProjectedCommit {
        projected_id: "after-native".into(),
        logical_commit_id: "after-native".into(),
        parent_projected_id: Some(native_oid.clone()),
        author: None,
        message: "after native".into(),
        changes: vec![projected_change("/request.txt", "after native\n")],
        materialization: ProjectionMaterialization::Generate,
    });
    let prefix = cached_projection_prefix(&engine, &incarnation, &extended)
        .unwrap()
        .unwrap();
    assert_eq!(prefix.commits, projection.commits.len());
    let extended_repo = projection_bare_repo_with_loader(
        &cache,
        Some(&incarnation),
        &extended,
        None,
        Some(prefix),
        |blob| Ok(blob.sha256.as_bytes().to_vec()),
    )
    .unwrap();
    assert_eq!(
        git_object_field(&extended_repo, "HEAD", "%H").unwrap(),
        scope_git::projection_head_oid(&extended).unwrap().unwrap()
    );
    assert!(!git_object_exists(&extended_repo, &private_oid));

    let mut corrupted = projection;
    let ProjectionMaterialization::PreserveGitCommit { tree_oid, .. } =
        &mut corrupted.commits[1].materialization
    else {
        panic!("expected preserved commit")
    };
    *tree_oid = "0000000000000000000000000000000000000000".to_string();
    let error =
        projection_bare_repo_with_loader(&cache, None, &corrupted, Some(&source), None, |blob| {
            Ok(blob.sha256.as_bytes().to_vec())
        })
        .unwrap_err();
    assert!(error.operator_diagnostic().contains("tree does not match"));
    assert_eq!(error.public_message(), "Scope hit an internal error.");

    let _ = fs::remove_dir_all(root);
}

fn projected_change(path: &str, content: &str) -> ProjectedChange {
    let mut git_blob = Sha1::new();
    git_blob.update(format!("blob {}\0", content.len()).as_bytes());
    git_blob.update(content.as_bytes());
    ProjectedChange {
        path: ScopePath::parse(path).unwrap(),
        new_content: Some(SourceBlob {
            content_ref: ContentRef::blob_sha256(content),
            sha256: content.to_string(),
            git_oid: hex::encode(git_blob.finalize()),
            git_file_mode: "100644".to_string(),
            size_bytes: content.len() as u64,
        }),
        visibility: Visibility::Public,
    }
}

fn projection_tree_path(path: &str) -> GitTreePath {
    GitTreePath::from_scope_path(&ScopePath::parse(path).unwrap()).unwrap()
}

fn git_object_exists(repo: &FsPath, oid: &str) -> bool {
    git_process_output_with_timeout(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo)
            .arg("cat-file")
            .arg("-e")
            .arg(format!("{oid}^{{object}}")),
        None,
        RuntimeBudgets::default_git_command_timeout(),
    )
    .unwrap()
    .status
    .success()
}

#[test]
fn generated_projection_reuses_only_a_matching_history_prefix() {
    use crate::git::repository_engine::RepositoryEngine;
    use std::cell::Cell;
    let root = tempfile::tempdir().unwrap();
    let engine = RepositoryEngine::new(root.path().to_path_buf(), 100 * 1024 * 1024).unwrap();
    let incarnation = RepositoryIncarnation::new("owner/repo", "repoi_original").unwrap();
    let mut projection = Projection {
        repo_id: "owner/repo".into(),
        view_key: ProjectionViewKey::Public,
        commits: vec![ProjectedCommit {
            projected_id: "initial".into(),
            logical_commit_id: "initial".into(),
            parent_projected_id: None,
            author: None,
            message: "initial".into(),
            changes: (0..40)
                .map(|i| projected_change(&format!("/file-{i}.txt"), &format!("initial {i}\n")))
                .collect(),
            materialization: ProjectionMaterialization::Generate,
        }],
    };
    for i in 1..20 {
        projection.commits.push(ProjectedCommit {
            projected_id: format!("commit-{i}"),
            logical_commit_id: format!("commit-{i}"),
            parent_projected_id: Some(projection.commits.last().unwrap().projected_id.clone()),
            author: None,
            message: format!("edit {i}"),
            changes: vec![projected_change("/file-0.txt", &format!("edit {i}\n"))],
            materialization: ProjectionMaterialization::Generate,
        });
    }
    let loaded = Cell::new(0);
    let bytes = Cell::new(0);
    let loader = |blob: &SourceBlob| {
        loaded.set(loaded.get() + 1);
        bytes.set(bytes.get() + blob.sha256.len());
        Ok(blob.sha256.as_bytes().to_vec())
    };
    let cold_started = std::time::Instant::now();
    let base = projection_bare_repo_with_loader(
        root.path(),
        Some(&incarnation),
        &projection,
        None,
        None,
        loader,
    )
    .unwrap();
    let cold_elapsed = cold_started.elapsed();
    assert_eq!(
        loaded.get(),
        59,
        "only changed blobs are loaded, not every file at every commit"
    );
    let cold_bytes = bytes.get();
    assert_eq!(
        git_object_field(&base, "HEAD", "%H").unwrap(),
        scope_git::projection_head_oid(&projection)
            .unwrap()
            .unwrap()
    );

    projection.commits.push(ProjectedCommit {
        projected_id: "append".into(),
        logical_commit_id: "append".into(),
        parent_projected_id: Some("commit-19".into()),
        author: None,
        message: "append".into(),
        changes: vec![projected_change("/file-0.txt", "appended\n")],
        materialization: ProjectionMaterialization::Generate,
    });
    let prefix = cached_projection_prefix(&engine, &incarnation, &projection)
        .unwrap()
        .unwrap();
    assert_eq!(prefix.commits, 20);
    loaded.set(0);
    bytes.set(0);
    let warm_started = std::time::Instant::now();
    let appended = projection_bare_repo_with_loader(
        root.path(),
        Some(&incarnation),
        &projection,
        None,
        Some(prefix),
        loader,
    )
    .unwrap();
    let warm_elapsed = warm_started.elapsed();
    assert_eq!(loaded.get(), 1);
    assert_eq!(
        git_object_field(&appended, "HEAD", "%H").unwrap(),
        scope_git::projection_head_oid(&projection)
            .unwrap()
            .unwrap()
    );
    assert!(
        !appended.join("objects/info/alternates").exists(),
        "prefix can be evicted independently"
    );
    fs::remove_dir_all(&base).unwrap();
    git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&appended)
            .args(["fsck", "--no-dangling"]),
        None,
    )
    .unwrap();
    eprintln!(
        "projection proof: cold={} blobs / {} bytes / {:?}; append={} blob / {} bytes / {:?}; base={}; append={}",
        59,
        cold_bytes,
        cold_elapsed,
        loaded.get(),
        bytes.get(),
        warm_elapsed,
        base.display(),
        appended.display()
    );

    let mut mode_change = projected_change("/file-0.txt", "appended\n");
    mode_change.new_content.as_mut().unwrap().git_file_mode = "100755".into();
    projection.commits.push(ProjectedCommit {
        projected_id: "mode".into(),
        logical_commit_id: "mode".into(),
        parent_projected_id: Some("append".into()),
        author: None,
        message: "mode only".into(),
        changes: vec![mode_change],
        materialization: ProjectionMaterialization::Generate,
    });
    let prefix = cached_projection_prefix(&engine, &incarnation, &projection)
        .unwrap()
        .unwrap();
    assert_eq!(prefix.commits, 21);
    let mode_only = projection_bare_repo_with_loader(
        root.path(),
        Some(&incarnation),
        &projection,
        None,
        Some(prefix),
        |_| panic!("mode-only append must reuse the verified blob"),
    )
    .unwrap();
    assert_eq!(
        git_object_field(&mode_only, "HEAD", "%H").unwrap(),
        scope_git::projection_head_oid(&projection)
            .unwrap()
            .unwrap()
    );

    // A visibility rewrite changes an earlier delta, so no previous prefix is valid.
    projection.commits[0].changes.remove(1);
    assert!(
        cached_projection_prefix(&engine, &incarnation, &projection)
            .unwrap()
            .is_none()
    );
    let rewritten = projection_bare_repo_with_loader(
        root.path(),
        Some(&incarnation),
        &projection,
        None,
        None,
        loader,
    )
    .unwrap();
    assert_eq!(
        git_object_field(&rewritten, "HEAD", "%H").unwrap(),
        scope_git::projection_head_oid(&projection)
            .unwrap()
            .unwrap()
    );
    let other = RepositoryIncarnation::new("owner/repo", "repoi_recreated").unwrap();
    assert!(
        cached_projection_prefix(&engine, &other, &projection)
            .unwrap()
            .is_none()
    );
}

#[test]
fn incremental_index_handles_deletes_modes_and_directory_replacement() {
    let root = tempfile::tempdir().unwrap();
    let mut executable = projected_change("/bin/run", "#!/bin/sh\n");
    executable.new_content.as_mut().unwrap().git_file_mode = "100755".into();
    let changes = vec![
        vec![projected_change("/path", "original"), executable],
        vec![
            ProjectedChange {
                path: ScopePath::parse("/path").unwrap(),
                new_content: None,
                visibility: Visibility::Public,
            },
            projected_change("/path/child", "child"),
        ],
        vec![
            ProjectedChange {
                path: ScopePath::parse("/path/child").unwrap(),
                new_content: None,
                visibility: Visibility::Public,
            },
            projected_change("/path", "replacement"),
        ],
    ];
    let projection = Projection {
        repo_id: "repo".into(),
        view_key: ProjectionViewKey::Public,
        commits: changes
            .into_iter()
            .enumerate()
            .map(|(i, changes)| ProjectedCommit {
                projected_id: i.to_string(),
                logical_commit_id: i.to_string(),
                parent_projected_id: i.checked_sub(1).map(|p| p.to_string()),
                author: None,
                message: i.to_string(),
                changes,
                materialization: ProjectionMaterialization::Generate,
            })
            .collect(),
    };
    let repo = projection_bare_repo_with_loader(root.path(), None, &projection, None, None, |b| {
        Ok(b.sha256.as_bytes().to_vec())
    })
    .unwrap();
    assert_eq!(
        git_object_field(&repo, "HEAD", "%H").unwrap(),
        scope_git::projection_head_oid(&projection)
            .unwrap()
            .unwrap()
    );
}
