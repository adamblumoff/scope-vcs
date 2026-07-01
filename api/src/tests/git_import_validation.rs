use super::*;

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

    let state = test_state_with_repo();
    let error = git_tree_files(&state, TEST_REPO_ID, &repo, "HEAD").unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert!(error.message.contains("unsupported Git tree entry"));
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn pushed_tree_rejects_non_utf8_blobs_before_pending_import() {
    let repo = temp_git_repo("binary-test");
    let binary = [0xff, 0x00, 0x61];
    fs::write(repo.join("image.bin"), binary).unwrap();
    run_git(Some(&repo), &["add", "image.bin"], "add binary").unwrap();
    commit_all(&repo, "binary");

    let state = test_state_with_repo();
    let error = git_tree_files(&state, TEST_REPO_ID, &repo, "HEAD").unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert!(error.message.contains("valid UTF-8 text"));
    assert!(!MemoryObjectStore::new().contains_bytes(&binary));
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn pushed_tree_cleans_uploaded_blobs_when_later_blob_is_invalid() {
    let repo = temp_git_repo("binary-cleanup-test");
    let valid = format!(
        "valid before binary cleanup {} {}",
        std::process::id(),
        unix_now()
    );
    fs::write(repo.join("a.txt"), &valid).unwrap();
    fs::write(repo.join("image.bin"), [0xff, 0x00, 0x61]).unwrap();
    run_git(Some(&repo), &["add", "-A"], "add mixed blobs").unwrap();
    commit_all(&repo, "mixed blobs");

    let state = test_state_with_repo();
    let error = git_tree_files(&state, TEST_REPO_ID, &repo, "HEAD").unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert!(error.message.contains("valid UTF-8 text"));
    assert!(!MemoryObjectStore::new().contains_bytes(valid.as_bytes()));
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn pushed_tree_accepts_executable_text_files() {
    let repo = temp_git_repo("mode-test");
    fs::write(repo.join("script.sh"), "#!/bin/sh\necho hi\n").unwrap();
    run_git(Some(&repo), &["add", "script.sh"], "add script").unwrap();
    run_git(
        Some(&repo),
        &["update-index", "--chmod=+x", "script.sh"],
        "make script executable",
    )
    .unwrap();
    commit_all(&repo, "executable");

    let state = test_state_with_repo();
    let files = git_tree_files(&state, TEST_REPO_ID, &repo, "HEAD").unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "script.sh");
    assert_eq!(files[0].mode, "100755");
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn pushed_tree_rejects_paths_scope_would_normalize_or_git_cannot_serve() {
    validate_pushed_file_path("docs/read me.md").unwrap();
    for path in [
        "README.md ",
        "dir\\file.txt",
        "line\nbreak.txt",
        "./README.md",
        "docs/../README.md",
    ] {
        let error = validate_pushed_file_path(path).unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }
}
