use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let mut path = env::temp_dir();
        path.push(format!(
            "scope-cli-push-{label}-{}-{}",
            std::process::id(),
            unix_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn push_help_exposes_remote_option() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["push", "--help"])
        .output()
        .unwrap();

    assert_success(&output, "scope push --help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--remote <REMOTE>"), "{stdout}");
}

#[test]
fn push_refuses_non_git_directory_before_login() {
    let dir = TempDir::new("non-git");
    let output = scope_command(dir.path()).args(["push"]).output().unwrap();

    assert_failure(&output, "scope push outside git repo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("run scope push from inside an existing Git repository"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn push_refuses_git_repo_without_head_before_login() {
    let dir = TempDir::new("no-head");
    run_git(dir.path(), ["-c", "init.defaultBranch=main", "init"]);

    let output = scope_command(dir.path()).args(["push"]).output().unwrap();

    assert_failure(&output, "scope push without HEAD");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("create at least one Git commit before running scope push"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn push_refuses_missing_config_before_remote_lookup() {
    let dir = TempDir::new("missing-config");
    create_repo_with_readme(dir.path());

    let output = scope_command(dir.path())
        .args(["push", "--yes"])
        .output()
        .unwrap();

    assert_failure(&output, "scope push without config");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("commit .scope/repo.json before running scope push"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("Scope remote 'scope' is not configured"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn push_refuses_invalid_config_before_remote_lookup() {
    let dir = TempDir::new("invalid-config");
    create_repo_with_readme(dir.path());
    fs::create_dir_all(dir.path().join(".scope")).unwrap();
    fs::write(
        dir.path().join(".scope/repo.json"),
        r#"{
  "kind": "wrong",
  "version": 1,
  "visibility": {
    "default": "private",
    "rules": []
  }
}
"#,
    )
    .unwrap();
    run_git(dir.path(), ["add", ".scope/repo.json"]);
    commit_all(dir.path(), "add invalid config");

    let output = scope_command(dir.path())
        .args(["push", "--yes"])
        .output()
        .unwrap();

    assert_failure(&output, "scope push with invalid config");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("repo config kind must be scope.repo-config"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("Scope remote 'scope' is not configured"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn push_refuses_uncommitted_config_before_remote_lookup() {
    let dir = TempDir::new("uncommitted-config");
    create_repo_with_readme(dir.path());
    write_valid_config(dir.path());

    let output = scope_command(dir.path())
        .args(["push", "--yes"])
        .output()
        .unwrap();

    assert_failure(&output, "scope push with uncommitted config");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            ".scope/repo.json has uncommitted changes; commit it before running scope push"
        ),
        "{stderr}"
    );
    assert!(
        !stderr.contains("Scope remote 'scope' is not configured"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn push_warns_on_dirty_working_tree_before_remote_lookup_failure() {
    let dir = TempDir::new("dirty");
    create_repo_with_head(dir.path());
    fs::write(dir.path().join("dirty.txt"), "uncommitted\n").unwrap();

    let output = scope_command(dir.path())
        .args(["push", "--yes"])
        .output()
        .unwrap();

    assert_failure(&output, "scope push without remote");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Working tree has uncommitted changes."),
        "{stderr}"
    );
    assert!(
        stderr.contains("Only committed HEAD will be pushed to Scope."),
        "{stderr}"
    );
    assert!(
        stderr.contains("Scope remote 'scope' is not configured. Run: scope init"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

fn scope_command(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scope"));
    command.current_dir(cwd);
    command.env("SCOPE_API_URL", unique_api_url());
    command
}

fn create_repo_with_head(cwd: &Path) {
    create_repo_with_readme(cwd);
    write_valid_config(cwd);
    run_git(cwd, ["add", ".scope/repo.json"]);
    commit_all(cwd, "add scope config");
}

fn create_repo_with_readme(cwd: &Path) {
    run_git(cwd, ["-c", "init.defaultBranch=main", "init"]);
    fs::write(cwd.join("README.md"), "initial\n").unwrap();
    run_git(cwd, ["add", "README.md"]);
    commit_all(cwd, "initial");
}

fn write_valid_config(cwd: &Path) {
    fs::create_dir_all(cwd.join(".scope")).unwrap();
    fs::write(
        cwd.join(".scope/repo.json"),
        r#"{
  "kind": "scope.repo-config",
  "version": 1,
  "visibility": {
    "default": "private",
    "rules": []
  },
  "history": {
    "rewrites": []
  }
}
"#,
    )
    .unwrap();
}

fn commit_all(cwd: &Path, message: &str) {
    run_git(
        cwd,
        [
            "-c",
            "user.email=scope@example.test",
            "-c",
            "user.name=Scope Test",
            "commit",
            "-m",
            message,
        ],
    );
}

fn run_git<const N: usize>(cwd: &Path, args: [&str; N]) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert_success(&output, "git");
}

fn assert_success(output: &Output, command: &str) {
    assert!(
        output.status.success(),
        "{command} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure(output: &Output, command: &str) {
    assert!(
        !output.status.success(),
        "{command} succeeded unexpectedly\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn unique_api_url() -> String {
    format!("http://127.0.0.1:9/scope-cli-test-{}", unix_nanos())
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}
