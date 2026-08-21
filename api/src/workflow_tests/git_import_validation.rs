use super::*;
use std::process::Command;

const LARGE_REPOSITORY_FILE_BYTES: u64 = 24 * 1024 * 1024;
const LARGE_REPOSITORY_FILE_COUNT: usize = 5;

fn add_shared_large_files(repo: &FsPath) {
    let first_path = repo.join("large-0.bin");
    let large = fs::File::create(&first_path).unwrap();
    large.set_len(LARGE_REPOSITORY_FILE_BYTES).unwrap();
    drop(large);
    for index in 1..LARGE_REPOSITORY_FILE_COUNT {
        fs::hard_link(&first_path, repo.join(format!("large-{index}.bin"))).unwrap();
    }
    run_git(Some(repo), &["add", "."], "add large files").unwrap();
}

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
    assert!(error.public_message().contains("reserved .git component"));
}

#[tokio::test]
async fn receive_pack_rejects_nested_windows_device_path_before_durable_side_effects() {
    let state = test_state_with_repo();
    let repo = temp_git_repo("reserved-windows-device-test");
    commit_all(&repo, "initial");
    let parent = git_stdout_text(&repo, &["rev-parse", "HEAD"], "read parent commit")
        .unwrap()
        .trim()
        .to_string();
    let malicious_blob = git_object_from_stdin(&repo, &["hash-object", "-w", "--stdin"], b"bad");
    let nested_tree = git_object_from_stdin(
        &repo,
        &["mktree"],
        format!("100644 blob {malicious_blob}\tCON.txt\n").as_bytes(),
    );
    let mut root_entries = git_stdout_text(&repo, &["ls-tree", "HEAD"], "read root tree")
        .unwrap()
        .into_bytes();
    root_entries.extend_from_slice(format!("040000 tree {nested_tree}\tdocs\n").as_bytes());
    let root_tree = git_object_from_stdin(&repo, &["mktree"], &root_entries);
    let commit = git_command_output(
        Command::new("git")
            .arg("-C")
            .arg(repo.as_ref())
            .arg("commit-tree")
            .arg(root_tree)
            .arg("-p")
            .arg(parent)
            .env("GIT_AUTHOR_NAME", "Scope Test")
            .env("GIT_AUTHOR_EMAIL", "scope-test@example.test")
            .env("GIT_COMMITTER_NAME", "Scope Test")
            .env("GIT_COMMITTER_EMAIL", "scope-test@example.test"),
        Some(b"crafted Windows device path\n"),
    )
    .unwrap();
    let commit = String::from_utf8(commit).unwrap().trim().to_string();
    run_git(
        Some(&repo),
        &["update-ref", "refs/heads/main", &commit],
        "install crafted commit",
    )
    .unwrap();
    let object_count = state.test_object_store.object_count();
    let refs = git_refs(&repo).unwrap();
    let metadata = serde_json::to_value(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap(),
    )
    .unwrap();

    let error = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &repo,
        &test_owner_id(),
        repo_config(Visibility::Public),
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert!(
        error
            .public_message()
            .contains("reserved Windows device name")
    );
    assert_eq!(state.test_object_store.object_count(), object_count);
    assert_eq!(git_refs(&repo).unwrap(), refs);
    assert_eq!(
        serde_json::to_value(
            find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
                .await
                .unwrap()
        )
        .unwrap(),
        metadata
    );
}

#[tokio::test]
async fn receive_pack_rejects_windows_device_path_removed_before_the_new_head() {
    let state = test_state_with_repo();
    let repo = temp_git_repo("intermediate-windows-device-test");
    commit_all(&repo, "initial");
    fs::write(repo.join("CON.txt"), "transient").unwrap();
    run_git(Some(&repo), &["add", "CON.txt"], "stage reserved path").unwrap();
    commit_all(&repo, "add reserved path");
    run_git(Some(&repo), &["rm", "CON.txt"], "remove reserved path").unwrap();
    commit_all(&repo, "remove reserved path");

    let error = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &repo,
        &test_owner_id(),
        repo_config(Visibility::Public),
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert!(
        error
            .public_message()
            .contains("reserved Windows device name")
    );
    assert_eq!(state.test_object_store.object_count(), 0);
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
    assert!(
        error
            .public_message()
            .contains("unsupported Git tree entry")
    );
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
    assert!(error.public_message().contains("video.bin"));
    assert!(
        error
            .public_message()
            .contains(&MAX_PENDING_IMPORT_BLOB_BYTES.to_string())
    );
}

#[test]
fn pushed_tree_does_not_cap_total_repository_bytes() {
    let repo = temp_git_repo("large-repository-test");
    add_shared_large_files(&repo);
    commit_all(&repo, "large repository");

    validate_pushed_tree(&repo, "HEAD").unwrap();
}

#[tokio::test]
async fn pushed_delta_does_not_cap_total_changed_bytes() {
    let state = test_state_with_repo();
    replace_test_repo(&state, repo_with_readme(&state)).await;
    let staging_repo = published_staging_repo(&state).await;
    let clone = clone_test_repo(&staging_repo, "large-delta-clone", false);
    add_shared_large_files(&clone);
    commit_all(&clone, "large delta");
    run_git(
        Some(&clone),
        &["push", "origin", DEFAULT_GIT_BRANCH],
        "push large delta",
    )
    .unwrap();

    let update = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &staging_repo,
        &test_owner_id(),
        repo_config(Visibility::Public),
    )
    .await
    .unwrap();

    let large_changes = update
        .changes
        .iter()
        .filter(|change| change.path.as_str().starts_with("/large-"))
        .collect::<Vec<_>>();
    assert_eq!(large_changes.len(), LARGE_REPOSITORY_FILE_COUNT);
    assert_eq!(
        large_changes
            .iter()
            .filter_map(|change| change.content.as_ref())
            .map(|content| content.size_bytes)
            .sum::<u64>(),
        LARGE_REPOSITORY_FILE_BYTES * LARGE_REPOSITORY_FILE_COUNT as u64
    );
    let _ = fs::remove_dir_all(staging_repo);
}

#[test]
fn pushed_tree_rejects_paths_scope_would_normalize_or_git_cannot_serve() {
    validate_pushed_file_path("docs/read me.md").unwrap();
    validate_pushed_file_path(".scope/RULES.md").unwrap();
    validate_pushed_file_path(".scope/runs/test.yml").unwrap();
    validate_pushed_file_path(".scope/runs/test-api.yaml").unwrap();
    validate_pushed_file_path(".scope/images/checks/Dockerfile").unwrap();
    validate_pushed_file_path(".scope/images/checks/.dockerignore").unwrap();
    validate_pushed_file_path(".scope/images/checks/scripts/install.sh").unwrap();
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
        ".scope/images",
        ".scope/images/Dockerfile",
        ".scope/images/Checks/Dockerfile",
        ".scope/images/checks--api/Dockerfile",
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
    assert!(
        error
            .public_message()
            .contains("must contain .scope/RULES.md")
    );
}

fn git_object_from_stdin(repo: &FsPath, args: &[&str], stdin: &[u8]) -> String {
    let output = git_command_output(
        Command::new("git").arg("-C").arg(repo).args(args),
        Some(stdin),
    )
    .unwrap();
    String::from_utf8(output).unwrap().trim().to_string()
}
