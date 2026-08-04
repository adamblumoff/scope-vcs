use super::*;
use std::process::Command;

#[test]
fn pushed_tree_rejects_case_insensitive_dot_git_from_a_raw_tree_object() {
    let repo = temp_git_repo("reserved-dot-git-test");
    commit_all(&repo, "initial");
    let mut root_entries = git_stdout_text(&repo, &["ls-tree", "HEAD"], "read root tree")
        .unwrap()
        .into_bytes();
    let malicious_blob = git_command_output(
        Command::new("git")
            .arg("-C")
            .arg(repo.as_ref())
            .arg("hash-object")
            .arg("-w")
            .arg("--stdin"),
        Some(b"malicious"),
    )
    .unwrap();
    root_entries.extend_from_slice(
        format!(
            "100644 blob {}\t.GiT\n",
            String::from_utf8(malicious_blob).unwrap().trim()
        )
        .as_bytes(),
    );
    let tree = git_command_output(
        Command::new("git")
            .arg("-C")
            .arg(repo.as_ref())
            .arg("mktree"),
        Some(&root_entries),
    )
    .unwrap();
    let commit = git_command_output(
        Command::new("git")
            .arg("-C")
            .arg(repo.as_ref())
            .arg("commit-tree")
            .arg(String::from_utf8(tree).unwrap().trim())
            .env("GIT_AUTHOR_NAME", "Scope Test")
            .env("GIT_AUTHOR_EMAIL", "scope-test@example.test")
            .env("GIT_COMMITTER_NAME", "Scope Test")
            .env("GIT_COMMITTER_EMAIL", "scope-test@example.test"),
        Some(b"crafted reserved path\n"),
    )
    .unwrap();
    let commit = String::from_utf8(commit).unwrap();

    let error = validate_pushed_tree(&repo, commit.trim()).unwrap_err();

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert!(error.message().contains("reserved .git component"));
}

#[test]
fn pushed_tree_rejects_gitlinks_instead_of_dropping_them() {
    let repo = temp_git_repo("gitlink-test");
    fs::write(repo.join("README.md"), "hello").unwrap();
    run_git(Some(&repo), &["add", "README.md"], "add readme").unwrap();
    commit_all(&repo, "initial");
    let commit = git_stdout_text(&repo, &["rev-parse", "HEAD"], "read head")
        .unwrap()
        .trim()
        .to_string();
    run_git(
        Some(&repo),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{commit},vendor/submodule"),
        ],
        "add gitlink",
    )
    .unwrap();
    commit_all(&repo, "add gitlink");

    let error = validate_pushed_tree(&repo, "HEAD").unwrap_err();

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert!(error.message().contains("unsupported Git tree entry"));
}

#[test]
fn oversized_binary_push_names_path_and_limit() {
    let repo = temp_git_repo("oversized-binary-test");
    let large_path = repo.join("video.bin");
    let large = fs::File::create(&large_path).unwrap();
    large
        .set_len((MAX_PENDING_IMPORT_BLOB_BYTES + 1) as u64)
        .unwrap();
    drop(large);
    run_git(Some(&repo), &["add", "video.bin"], "add oversized binary").unwrap();
    commit_all(&repo, "oversized binary");

    let error = validate_pushed_tree(&repo, "HEAD").unwrap_err();

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert!(error.message().contains("video.bin"));
    assert!(
        error
            .message()
            .contains(&MAX_PENDING_IMPORT_BLOB_BYTES.to_string())
    );
}

#[test]
fn pushed_tree_rejects_paths_scope_would_normalize_or_git_cannot_serve() {
    validate_pushed_file_path("docs/read me.md").unwrap();
    validate_pushed_file_path(".scope/RULES.md").unwrap();
    validate_pushed_file_path(".scope/runs/test.yml").unwrap();
    validate_pushed_file_path(".scope/runs/test-api.yaml").unwrap();
    for path in [
        "README.md ",
        "dir\\file.txt",
        "line\nbreak.txt",
        "./README.md",
        "docs/../README.md",
        ".git/config",
        "vendor/.GIT/index",
        ".scope",
        ".scope/repo.json",
        ".scope/anything.json",
        ".scope/runs/Test.yml",
        ".scope/runs/test.json",
        ".scope/runs/nested/test.yml",
    ] {
        let error = validate_pushed_file_path(path).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }
}

#[test]
fn pushed_tree_requires_canonical_repo_rules() {
    let repo = temp_git_repo("missing-rules-test");
    fs::remove_file(repo.join(".scope/RULES.md")).unwrap();
    run_git(
        Some(&repo),
        &["rm", "--cached", ".scope/RULES.md"],
        "unstage rules",
    )
    .unwrap();
    fs::write(repo.join("README.md"), "hello").unwrap();
    run_git(Some(&repo), &["add", "README.md"], "add readme").unwrap();
    run_git(
        Some(&repo),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "commit",
            "-m",
            "missing rules",
        ],
        "commit without rules",
    )
    .unwrap();

    let error = validate_pushed_tree(&repo, "HEAD").unwrap_err();

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert!(error.message().contains("must contain .scope/RULES.md"));
}
