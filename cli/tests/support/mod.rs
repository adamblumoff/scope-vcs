use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(label: &str) -> Self {
        let path = env::temp_dir().join(format!(
            "scope-cli-{label}-{}-{}",
            std::process::id(),
            unix_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn scope_command(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scope"));
    command.current_dir(cwd);
    command.env(
        "SCOPE_API_URL",
        format!("http://127.0.0.1:9/scope-cli-test-{}", unix_nanos()),
    );
    command
}

#[allow(dead_code)]
pub fn scope_failure<const N: usize>(cwd: &Path, args: [&str; N], expected: &str) -> String {
    let output = scope_command(cwd).args(args).output().unwrap();
    assert_failure(&output, "scope command");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains(expected), "{stderr}");
    assert!(!stderr.contains("start browser login"), "{stderr}");
    stderr
}

pub fn create_repo_with_head(cwd: &Path) {
    run_git(cwd, ["-c", "init.defaultBranch=main", "init"]);
    fs::write(cwd.join("README.md"), "initial\n").unwrap();
    run_git(cwd, ["add", "README.md"]);
    commit_all(cwd, "initial");
}

pub fn commit_all(cwd: &Path, message: &str) {
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

pub fn run_git<const N: usize>(cwd: &Path, args: [&str; N]) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn assert_failure(output: &Output, action: &str) {
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
