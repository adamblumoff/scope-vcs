use super::*;

#[tokio::test]
async fn pushed_tree_rejects_gitlinks_instead_of_dropping_them() {
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
    let error = git_tree_files(&state, TEST_REPO_ID, &repo, "HEAD")
        .await
        .unwrap_err();

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert!(error.message().contains("unsupported Git tree entry"));
}

#[tokio::test]
async fn pushed_tree_accepts_binary_blobs() {
    let repo = temp_git_repo("binary-test");
    let binary = [0xff, 0x00, 0x61];
    fs::write(repo.join("image.bin"), binary).unwrap();
    run_git(Some(&repo), &["add", "image.bin"], "add binary").unwrap();
    commit_all(&repo, "binary");

    let state = test_state_with_repo();
    let files = git_tree_files(&state, TEST_REPO_ID, &repo, "HEAD")
        .await
        .unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0.path, "image.bin");
    assert!(MemoryObjectStore::new().contains_bytes(&binary));
}

#[tokio::test]
async fn oversized_binary_push_names_path_and_limit() {
    let repo = temp_git_repo("oversized-binary-test");
    let large_path = repo.join("video.bin");
    let large = fs::File::create(&large_path).unwrap();
    large
        .set_len((MAX_PENDING_IMPORT_BLOB_BYTES + 1) as u64)
        .unwrap();
    drop(large);
    run_git(Some(&repo), &["add", "video.bin"], "add oversized binary").unwrap();
    commit_all(&repo, "oversized binary");

    let state = test_state_with_repo();
    let error = git_tree_files(&state, TEST_REPO_ID, &repo, "HEAD")
        .await
        .unwrap_err();

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert!(error.message().contains("video.bin"));
    assert!(
        error
            .message()
            .contains(&MAX_PENDING_IMPORT_BLOB_BYTES.to_string())
    );
}

#[tokio::test]
async fn pushed_tree_accepts_executable_text_files() {
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
    let files = git_tree_files(&state, TEST_REPO_ID, &repo, "HEAD")
        .await
        .unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0.path, "script.sh");
    assert_eq!(files[0].0.mode, "100755");
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
        ".scope",
        ".scope/repo.json",
        ".scope/anything.json",
    ] {
        let error = validate_pushed_file_path(path).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }
}
