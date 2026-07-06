use super::*;
use std::{
    env,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let mut path = env::temp_dir();
        path.push(format!(
            "scope-repo-config-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(unix)]
#[test]
fn write_rejects_symlinked_worktree_config_file() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new("file-symlink");
    fs::create_dir_all(dir.path.join(".scope")).unwrap();
    let target = dir.path.join("outside.json");
    fs::write(&target, default_repo_config_json()).unwrap();
    symlink(&target, dir.path.join(WORKTREE_CONFIG_PATH)).unwrap();
    let config = RepoConfig::parse_json(default_repo_config_json().as_bytes()).unwrap();

    let error = write_worktree_scope_repo_config(&dir.path, &config).unwrap_err();

    assert!(
        error
            .to_string()
            .contains(".scope/repo.json cannot be a symlink")
    );
}

#[cfg(unix)]
#[test]
fn write_rejects_symlinked_scope_directory() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new("dir-symlink");
    let target_dir = dir.path.join("outside");
    fs::create_dir_all(&target_dir).unwrap();
    symlink(&target_dir, dir.path.join(".scope")).unwrap();
    let config = RepoConfig::parse_json(default_repo_config_json().as_bytes()).unwrap();

    let error = write_worktree_scope_repo_config(&dir.path, &config).unwrap_err();

    assert!(
        error
            .to_string()
            .contains(".scope config directory cannot be a symlink")
    );
}

#[test]
fn write_replaces_existing_config_without_leaving_temp_file() {
    let dir = TempDir::new("atomic-write");
    fs::create_dir_all(dir.path.join(".scope")).unwrap();
    fs::write(dir.path.join(WORKTREE_CONFIG_PATH), "old config").unwrap();
    let config = RepoConfig::parse_json(default_repo_config_json().as_bytes()).unwrap();

    write_worktree_scope_repo_config(&dir.path, &config).unwrap();

    assert_eq!(
        fs::read_to_string(dir.path.join(WORKTREE_CONFIG_PATH)).unwrap(),
        canonical_repo_config_json(&config).unwrap()
    );
    let entries = fs::read_dir(dir.path.join(".scope"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec!["repo.json"]);
}

#[cfg(unix)]
#[test]
fn create_rejects_symlinked_worktree_config_file() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new("create-file-symlink");
    fs::create_dir_all(dir.path.join(".scope")).unwrap();
    let target = dir.path.join("outside.json");
    symlink(&target, dir.path.join(WORKTREE_CONFIG_PATH)).unwrap();

    let error = ensure_scope_repo_config_exists(&dir.path).unwrap_err();

    assert!(
        error
            .to_string()
            .contains(".scope/repo.json cannot be a symlink")
    );
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn create_rejects_symlinked_scope_directory() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new("create-dir-symlink");
    let target_dir = dir.path.join("outside");
    fs::create_dir_all(&target_dir).unwrap();
    symlink(&target_dir, dir.path.join(".scope")).unwrap();

    let error = ensure_scope_repo_config_exists(&dir.path).unwrap_err();

    assert!(
        error
            .to_string()
            .contains(".scope config directory cannot be a symlink")
    );
    assert!(!target_dir.join("repo.json").exists());
}
