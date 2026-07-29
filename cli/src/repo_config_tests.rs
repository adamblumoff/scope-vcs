use super::*;
use crate::test_support::TestDir as TempDir;

#[cfg(unix)]
#[test]
fn write_and_create_reject_symlinked_state_paths() {
    use std::os::unix::fs::symlink;

    for (operation, symlink_directory) in [
        ("write", false),
        ("create", false),
        ("write", true),
        ("create", true),
    ] {
        let dir = TempDir::git_repo(
            &format!(
                "{operation}-{}-symlink",
                if symlink_directory { "dir" } else { "file" }
            ),
            "main",
        );
        let paths = repo_state_paths(&dir.path).unwrap();
        let outside = dir.path.join("outside");
        if symlink_directory {
            fs::create_dir(&outside).unwrap();
            symlink(&outside, &paths.directory).unwrap();
        } else {
            fs::create_dir(&paths.directory).unwrap();
            fs::write(&outside, default_repo_config_json()).unwrap();
            symlink(&outside, &paths.config).unwrap();
        }

        let error = if operation == "write" {
            write_worktree_scope_repo_config(&dir.path, &default_scope_repo_config()).unwrap_err()
        } else {
            ensure_scope_repo_config_exists(&dir.path).unwrap_err()
        };
        assert!(error.to_string().contains(if symlink_directory {
            "Scope repo state directory cannot be a symlink"
        } else {
            "Scope repo config cannot be a symlink"
        }));
    }
}

#[test]
fn synced_config_lives_only_in_per_worktree_git_state() {
    let dir = TempDir::git_repo("state", "main");
    let config = default_scope_repo_config();

    write_worktree_scope_repo_config_with_base(&dir.path, &config).unwrap();
    let paths = repo_state_paths(&dir.path).unwrap();

    assert_eq!(
        load_worktree_scope_repo_config_base_hash(&dir.path).unwrap(),
        repo_config_fingerprint(&config).unwrap()
    );
    assert_eq!(repo_config_path(&dir.path).unwrap(), paths.config);
    assert!(paths.config.is_file());
    assert!(paths.state.is_file());
    assert!(!dir.path.join(".scope").exists());
    assert!(!dir.path.join(".gitignore").exists());
    assert!(
        !fs::read_to_string(dir.path.join(".git/info/exclude"))
            .unwrap_or_default()
            .lines()
            .any(|line| line.trim() == "/.scope/")
    );
}

#[test]
fn linked_worktrees_get_distinct_scope_state_directories() {
    let main = TempDir::git_repo("linked-main", "main");
    fs::write(main.path.join("README.md"), "initial\n").unwrap();
    main.run_git(["add", "README.md"]);
    main.run_git([
        "-c",
        "user.email=scope@example.test",
        "-c",
        "user.name=Scope Test",
        "commit",
        "-m",
        "initial",
    ]);
    let linked = main.path.join("linked");
    main.run_git(["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);

    ensure_scope_repo_config_exists(&main.path).unwrap();
    ensure_scope_repo_config_exists(&linked).unwrap();
    let main_path = repo_config_path(&main.path).unwrap();
    let linked_path = repo_config_path(&linked).unwrap();

    assert_ne!(main_path, linked_path);
    assert!(main_path.ends_with(".git/scope/repo.json"));
    assert!(linked_path.ends_with("worktrees/linked/scope/repo.json"));
    assert!(!main.path.join(".scope").exists());
    assert!(!linked.join(".scope").exists());
}
