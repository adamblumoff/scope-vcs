use super::*;
use scope_domain::{
    content_ref::ContentRef,
    policy::{ScopePath, Visibility},
    projection::{ProjectedChange, ProjectedCommit, ProjectionViewKey},
    store::SourceBlob,
};
use std::time::{SystemTime, UNIX_EPOCH};

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

    let repo = projection_bare_repo_with_loader(&root, &projection, None, |blob| {
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

    let repo = projection_bare_repo_with_loader(&root, &projection, None, |blob| {
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
    let materialization_error = projection_bare_repo_with_loader(&root, &projection, None, |_| {
        panic!("invalid paths must fail before loading content")
    })
    .unwrap_err();

    assert_eq!(materialization_error.message(), identity_error);
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

    let repo = projection_bare_repo_with_loader(&cache, &projection, Some(&source), |blob| {
        Ok(blob.sha256.as_bytes().to_vec())
    })
    .unwrap();
    assert_eq!(
        git_object_field(&repo, "refs/heads/main", "%H").unwrap(),
        native_oid
    );
    assert!(!git_object_exists(&repo, &private_oid));
    assert!(!git_object_exists(&repo, &secret_blob_oid));
    assert!(!git_object_exists(&repo, &canonical_merge_oid));

    let mut corrupted = projection;
    let ProjectionMaterialization::PreserveGitCommit { tree_oid, .. } =
        &mut corrupted.commits[1].materialization
    else {
        panic!("expected preserved commit")
    };
    *tree_oid = "0000000000000000000000000000000000000000".to_string();
    let error = projection_bare_repo_with_loader(&cache, &corrupted, Some(&source), |blob| {
        Ok(blob.sha256.as_bytes().to_vec())
    })
    .unwrap_err();
    assert!(
        error
            .message()
            .contains("tree does not match persisted provenance")
    );

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
