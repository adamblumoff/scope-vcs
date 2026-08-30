mod support;

use axum::{
    Json, Router,
    http::StatusCode,
    routing::{delete, get, post},
};
use std::{
    fs,
    net::TcpListener,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};
use support::*;
use tokio::sync::oneshot;

const REMOTE_URL: &str = "https://scope.example/git/adam/sample";

#[test]
fn init_configures_an_unborn_repository_for_its_first_push() {
    let dir = TempDir::new("unborn");
    run_git(dir.path(), ["-c", "init.defaultBranch=main", "init"]);
    fs::create_dir(dir.path().join(".codex")).unwrap();
    let config_dir = TempDir::new("unborn-config");
    let server = InitServer::start();

    let output = authenticated_init_command(dir.path(), config_dir.path(), &server.api_url)
        .args(["init", "--name", "sample"])
        .output()
        .unwrap();
    server.finish();

    assert_success(&output, "scope init in unborn repository");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains(
            "Create your first commit including the generated Scope files, then run: scope push"
        ),
        "{stdout}"
    );
    assert!(!stderr.contains("No such remote"), "{stderr}");
    assert!(dir.path().join(".scope/RULES.md").is_file());
    assert!(
        fs::read_to_string(dir.path().join("AGENTS.md"))
            .unwrap()
            .contains("Read and follow `.scope/RULES.md`")
    );
    assert!(dir.path().join(".git/scope/repo.json").is_file());
    assert_eq!(
        git_stdout(dir.path(), ["remote", "get-url", "scope"]),
        REMOTE_URL
    );
    assert!(
        !std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["rev-parse", "--verify", "HEAD"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn init_keeps_the_existing_committed_repository_flow() {
    let dir = TempDir::new("committed");
    create_repo_with_head(dir.path());
    let config_dir = TempDir::new("committed-config");
    let server = InitServer::start();

    let output = authenticated_init_command(dir.path(), config_dir.path(), &server.api_url)
        .args(["init", "--name", "sample"])
        .output()
        .unwrap();
    server.finish();

    assert_success(&output, "scope init in committed repository");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("Run: scope push"), "{stdout}");
    assert!(!stdout.contains("Create your first commit"), "{stdout}");
    assert!(!stderr.contains("No such remote"), "{stderr}");
    assert_eq!(
        git_stdout(dir.path(), ["remote", "get-url", "scope"]),
        REMOTE_URL
    );
}

#[test]
fn init_restores_the_remote_when_later_local_setup_fails() {
    let dir = TempDir::new("rollback");
    create_repo_with_head(dir.path());
    run_git(
        dir.path(),
        ["remote", "add", "scope", "https://old.scope.example/repo"],
    );
    fs::write(
        dir.path().join(".git/scope"),
        "blocks Scope state directory\n",
    )
    .unwrap();
    let config_dir = TempDir::new("rollback-config");
    let server = InitServer::start();

    let output = authenticated_init_command(dir.path(), config_dir.path(), &server.api_url)
        .args(["init", "--name", "sample"])
        .output()
        .unwrap();
    let rolled_back = server.finish();

    assert_failure(&output, "scope init with failed local config");
    assert!(
        rolled_back,
        "the created server repository was not rolled back"
    );
    assert_eq!(
        git_stdout(dir.path(), ["remote", "get-url", "scope"]),
        "https://old.scope.example/repo"
    );
}

#[test]
fn init_warns_on_dirty_working_tree_and_continues_to_auth() {
    let dir = TempDir::new("dirty");
    create_repo_with_head(dir.path());
    fs::write(dir.path().join("README.md"), "uncommitted\n").unwrap();

    let output = scope_command(dir.path())
        .args(["init", "--name", "sample"])
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

fn authenticated_init_command(
    cwd: &Path,
    config_dir: &Path,
    api_url: &str,
) -> std::process::Command {
    write_session(config_dir, api_url);
    let mut command = scope_command(cwd);
    command.env("SCOPE_API_URL", api_url);
    command.env("XDG_CONFIG_HOME", config_dir);
    command
}

fn write_session(config_dir: &Path, api_url: &str) {
    let key = api_url
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let directory = config_dir.join("scope/sessions");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join(format!("cli-session-{key}")), "test-token").unwrap();
}

fn git_stdout<const N: usize>(cwd: &Path, args: [&str; N]) -> String {
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn assert_success(output: &std::process::Output, action: &str) {
    assert!(
        output.status.success(),
        "{action} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

struct InitServer {
    api_url: String,
    rolled_back: Arc<AtomicBool>,
    stop: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

impl InitServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let (stop, stopped) = oneshot::channel();
        let rolled_back = Arc::new(AtomicBool::new(false));
        let rollback_state = rolled_back.clone();
        let handle = thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let app = Router::new()
                    .route("/v1/session", get(|| async { Json(session_response()) }))
                    .route("/v1/repos", post(|| async { Json(create_repo_response()) }))
                    .route(
                        "/v1/repos/adam/sample",
                        delete(move || {
                            let rollback_state = rollback_state.clone();
                            async move {
                                rollback_state.store(true, Ordering::SeqCst);
                                StatusCode::NO_CONTENT
                            }
                        }),
                    );
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = stopped.await;
                    })
                    .await
                    .unwrap();
            });
        });
        Self {
            api_url,
            rolled_back,
            stop,
            handle,
        }
    }

    fn finish(self) -> bool {
        let _ = self.stop.send(());
        self.handle.join().unwrap();
        self.rolled_back.load(Ordering::SeqCst)
    }
}

fn session_response() -> serde_json::Value {
    serde_json::json!({
        "identity": null,
        "user": {
            "id": "user_test",
            "handle": "adam",
            "email": "adam@example.test",
            "email_verified": true
        }
    })
}

fn create_repo_response() -> serde_json::Value {
    let repo = serde_json::json!({
        "id": "repo_test",
        "owner_handle": "adam",
        "name": "sample",
        "git_remote_url": REMOTE_URL,
        "lifecycle_state": "AwaitingFirstPush",
        "change_version": 1,
        "access": {
            "actor": "Owner",
            "can_read_private_files": true,
            "can_push": true,
            "can_change_file_visibility": true,
            "can_apply_changes": true,
            "can_manage_members": true,
            "can_delete_repo": true
        },
        "open_request_count": 0,
        "request_permissions": { "can_start_request": true }
    });
    serde_json::json!({
        "repo": repo,
        "init": {
            "repo": repo,
            "git_remote_url": REMOTE_URL,
            "remote_name": "scope",
            "push_branch": "main",
            "token": null,
            "push_token": null
        }
    })
}
