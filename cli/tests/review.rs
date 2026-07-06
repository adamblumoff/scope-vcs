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
            "scope-cli-review-{label}-{}-{}",
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
fn review_help_is_available() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["review", "--help"])
        .output()
        .unwrap();

    assert_success(&output, "scope review --help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: scope review"), "{stdout}");
}

#[test]
fn review_refuses_non_git_directory() {
    let dir = TempDir::new("non-git");
    let output = scope_command(dir.path()).args(["review"]).output().unwrap();

    assert_failure(&output, "scope review outside git repo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("run scope review from inside an existing Git repository"),
        "{stderr}"
    );
}

#[test]
fn review_requires_interactive_terminal_before_creating_config() {
    let dir = TempDir::new("non-tty");
    create_repo_with_head(dir.path());

    let output = scope_command(dir.path()).args(["review"]).output().unwrap();

    assert_failure(&output, "scope review without tty");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scope review requires an interactive terminal"),
        "{stderr}"
    );
    assert!(!dir.path().join(".scope/repo.json").exists());
}

#[test]
fn review_can_start_before_first_commit() {
    let dir = TempDir::new("no-head");
    run_git(dir.path(), ["-c", "init.defaultBranch=main", "init"]);

    let output = scope_command(dir.path()).args(["review"]).output().unwrap();

    assert_failure(&output, "scope review without HEAD");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scope review requires an interactive terminal"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("create at least one Git commit before running scope review"),
        "{stderr}"
    );
}

fn scope_command(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scope"));
    command.current_dir(cwd);
    command.env("SCOPE_API_URL", unique_api_url());
    command
}

fn create_repo_with_head(cwd: &Path) {
    run_git(cwd, ["-c", "init.defaultBranch=main", "init"]);
    fs::write(cwd.join("README.md"), "initial\n").unwrap();
    run_git(cwd, ["add", "README.md"]);
    run_git(
        cwd,
        [
            "-c",
            "user.email=scope@example.test",
            "-c",
            "user.name=Scope Test",
            "commit",
            "-m",
            "initial",
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
