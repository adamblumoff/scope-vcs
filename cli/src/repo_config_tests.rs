use super::*;
use crate::test_support::TestDir as TempDir;

#[cfg(unix)]
#[test]
fn write_and_create_reject_symlinked_config_paths() {
    use std::os::unix::fs::symlink;

    for (operation, symlink_directory) in [
        ("write", false),
        ("create", false),
        ("write", true),
        ("create", true),
    ] {
        let dir = TempDir::new(&format!(
            "{operation}-{}-symlink",
            if symlink_directory { "dir" } else { "file" }
        ));
        let outside = if symlink_directory {
            let path = dir.path.join("outside");
            fs::create_dir(&path).unwrap();
            symlink(&path, dir.path.join(".scope")).unwrap();
            path
        } else {
            fs::create_dir(dir.path.join(".scope")).unwrap();
            let path = dir.path.join("outside.json");
            if operation == "write" {
                fs::write(&path, default_repo_config_json()).unwrap();
            }
            symlink(&path, dir.path.join(WORKTREE_CONFIG_PATH)).unwrap();
            path
        };
        let error = if operation == "write" {
            write_worktree_scope_repo_config(&dir.path, &default_scope_repo_config()).unwrap_err()
        } else {
            ensure_scope_repo_config_exists(&dir.path).unwrap_err()
        };
        assert!(error.to_string().contains(if symlink_directory {
            ".scope config directory cannot be a symlink"
        } else {
            ".scope/repo.json cannot be a symlink"
        }));
        if operation == "create" {
            assert!(
                !if symlink_directory {
                    outside.join("repo.json")
                } else {
                    outside
                }
                .exists()
            );
        }
    }
}

#[test]
fn synced_config_writes_base_hash_and_only_locally_excludes_state() {
    let dir = TempDir::new("state");
    fs::create_dir_all(dir.path.join(".git/info")).unwrap();
    let config = default_scope_repo_config();

    write_worktree_scope_repo_config_with_base(&dir.path, &config).unwrap();

    assert_eq!(
        load_worktree_scope_repo_config_base_hash(&dir.path).unwrap(),
        repo_config_fingerprint(&config).unwrap()
    );
    assert!(!dir.path.join(".gitignore").exists());
    assert!(
        fs::read_to_string(dir.path.join(".git/info/exclude"))
            .unwrap()
            .lines()
            .any(|line| line == "/.scope/")
    );
}

#[test]
fn creating_config_uses_linked_worktree_git_exclude_path() {
    let main = TempDir::new("linked-main");
    main.run_git(["-c", "init.defaultBranch=main", "init"]);
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

    ensure_scope_repo_config_exists(&linked).unwrap();

    let exclude_path = git_info_exclude_path(&linked).unwrap().unwrap();
    assert!(
        fs::read_to_string(linked.join(".gitignore"))
            .unwrap()
            .lines()
            .any(|line| line == "/.scope/")
    );
    assert!(
        fs::read_to_string(exclude_path)
            .unwrap()
            .lines()
            .any(|line| line == "/.scope/")
    );
}
