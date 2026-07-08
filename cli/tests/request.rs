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
            "scope-cli-request-{label}-{}-{}",
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
fn request_help_exposes_branch_backed_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["request", "--help"])
        .output()
        .unwrap();

    assert_success(&output, "scope request --help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("start"), "{stdout}");
    assert!(stdout.contains("submit"), "{stdout}");
    assert!(stdout.contains("update"), "{stdout}");
    assert!(stdout.contains("sync"), "{stdout}");
    assert!(stdout.contains("status"), "{stdout}");
    assert!(stdout.contains("comment"), "{stdout}");
    assert!(stdout.contains("needs-response"), "{stdout}");
    assert!(stdout.contains("respond"), "{stdout}");
    assert!(stdout.contains("resolve"), "{stdout}");
    assert!(stdout.contains("merge"), "{stdout}");
}

#[test]
fn request_submit_help_exposes_title_and_stake() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["request", "submit", "--help"])
        .output()
        .unwrap();

    assert_success(&output, "scope request submit --help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--title <TITLE>"), "{stdout}");
    assert!(
        stdout.contains("--stake-credits <STAKE_CREDITS>"),
        "{stdout}"
    );
    assert!(stdout.contains("--remote <REMOTE>"), "{stdout}");
}

#[test]
fn request_start_refuses_non_git_directory_before_login() {
    let dir = TempDir::new("non-git");
    let output = scope_command(dir.path())
        .args(["request", "start"])
        .output()
        .unwrap();

    assert_failure(&output, "scope request start outside git repo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("run scope request start from inside an existing Git repository"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn request_sync_refuses_dirty_worktree_before_login() {
    let dir = TempDir::new("dirty");
    create_repo_with_head(dir.path());
    fs::write(dir.path().join("dirty.txt"), "uncommitted\n").unwrap();

    let output = scope_command(dir.path())
        .args(["request", "sync"])
        .output()
        .unwrap();

    assert_failure(&output, "scope request sync with dirty worktree");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("commit or stash local changes before running scope request sync"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn request_sync_refuses_unattached_branch_before_login() {
    let dir = TempDir::new("sync-unattached");
    create_repo_with_head(dir.path());

    let output = scope_command(dir.path())
        .args(["request", "sync"])
        .output()
        .unwrap();

    assert_failure(&output, "scope request sync on non-request branch");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scope request sync requires a Scope request branch"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn request_submit_refuses_branch_already_attached_to_request_before_login() {
    let dir = TempDir::new("attached");
    create_repo_with_head(dir.path());
    run_git(
        dir.path(),
        [
            "config",
            "--local",
            "branch.main.scopeRequestId",
            "req_existing",
        ],
    );

    let output = scope_command(dir.path())
        .args([
            "request",
            "submit",
            "--title",
            "second submit",
            "--stake-credits",
            "1",
        ])
        .output()
        .unwrap();

    assert_failure(&output, "scope request submit on attached branch");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "current branch is already attached to request req_existing; run scope request update req_existing instead"
        ),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn request_submit_refuses_detached_head_before_login() {
    let dir = TempDir::new("detached");
    create_repo_with_head(dir.path());
    run_git(dir.path(), ["checkout", "--detach"]);

    let output = scope_command(dir.path())
        .args([
            "request",
            "submit",
            "--title",
            "detached submit",
            "--stake-credits",
            "1",
        ])
        .output()
        .unwrap();

    assert_failure(&output, "scope request submit on detached head");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("request commands require a named local branch"),
        "{stderr}"
    );
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

fn scope_command(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scope"));
    command.current_dir(dir);
    command
}

fn create_repo_with_head(path: &Path) {
    run_git(path, ["-c", "init.defaultBranch=main", "init"]);
    fs::write(path.join("README.md"), "# sample\n").unwrap();
    run_git(path, ["add", "README.md"]);
    commit_all(path, "initial commit");
}

fn commit_all(path: &Path, message: &str) {
    run_git(
        path,
        [
            "-c",
            "user.name=Scope Tests",
            "-c",
            "user.email=scope-tests@example.com",
            "commit",
            "-m",
            message,
        ],
    );
}

fn run_git<const N: usize>(path: &Path, args: [&str; N]) {
    let output = Command::new("git")
        .current_dir(path)
        .args(args)
        .output()
        .unwrap();
    assert_success(&output, "git command");
}

fn assert_success(output: &Output, action: &str) {
    assert!(
        output.status.success(),
        "{action} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure(output: &Output, action: &str) {
    assert!(
        !output.status.success(),
        "{action} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}
