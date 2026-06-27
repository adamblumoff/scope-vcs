use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let mut path = env::temp_dir();
        path.push(format!(
            "scope-cli-init-{label}-{}-{}",
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
fn init_help_exposes_name_and_visibility_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["init", "--help"])
        .output()
        .unwrap();

    assert_success(&output, "scope init --help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--name <NAME>"), "{stdout}");
    assert!(stdout.contains("--public"), "{stdout}");
    assert!(stdout.contains("--private"), "{stdout}");
    assert!(!stdout.contains("[NAME]"), "{stdout}");
}

#[test]
fn init_refuses_non_git_directory_without_creating_repo() {
    let dir = TempDir::new("non-git");
    let output = scope_command(dir.path())
        .args(["init", "--name", "sample", "--private"])
        .output()
        .unwrap();

    assert_failure(&output, "scope init outside git repo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("run scope init from inside an existing Git repository"),
        "{stderr}"
    );
    assert!(!dir.path().join(".git").exists());
}

#[test]
fn init_refuses_git_repo_without_head() {
    let dir = TempDir::new("no-head");
    run_git(dir.path(), ["-c", "init.defaultBranch=main", "init"]);

    let output = scope_command(dir.path())
        .args(["init", "--name", "sample", "--private"])
        .output()
        .unwrap();

    assert_failure(&output, "scope init without HEAD");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("create at least one Git commit before running scope init"),
        "{stderr}"
    );
}

#[test]
fn init_prompts_for_default_visibility_when_flag_is_omitted() {
    let dir = TempDir::new("visibility-prompt");
    create_repo_with_head(dir.path());

    let output = scope_command_with_stdin(dir.path(), "\n", ["init", "--name", "sample"]);

    assert_failure(&output, "scope init without API");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Default visibility [Private]:"), "{stderr}");
    assert!(stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn init_warns_on_dirty_working_tree_and_continues_to_auth() {
    let dir = TempDir::new("dirty");
    create_repo_with_head(dir.path());
    fs::write(dir.path().join("dirty.txt"), "uncommitted\n").unwrap();

    let output = scope_command(dir.path())
        .args(["init", "--name", "sample", "--private"])
        .output()
        .unwrap();

    assert_failure(&output, "scope init with dirty working tree");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Working tree has uncommitted changes."),
        "{stderr}"
    );
    assert!(
        stderr.contains("Only committed HEAD will be pushed to Scope."),
        "{stderr}"
    );
    assert!(stderr.contains("start browser login"), "{stderr}");
}

fn scope_command(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scope"));
    command.current_dir(cwd);
    command.env("SCOPE_API_URL", unique_api_url());
    command
}

fn scope_command_with_stdin<const N: usize>(cwd: &Path, input: &str, args: [&str; N]) -> Output {
    let mut child = scope_command(cwd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
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
