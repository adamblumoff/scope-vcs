use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TestDir {
    pub(crate) path: PathBuf,
}

impl TestDir {
    pub(crate) fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "scope-cli-{label}-{}-{}",
            std::process::id(),
            NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub(crate) fn git_repo(label: &str, branch: &str) -> Self {
        let dir = Self::new(label);
        let status = Command::new("git")
            .current_dir(&dir.path)
            .args(["init", "--quiet", "-b", branch])
            .status()
            .unwrap();
        assert!(status.success());
        dir
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn run_git<const N: usize>(&self, args: [&str; N]) -> Output {
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
