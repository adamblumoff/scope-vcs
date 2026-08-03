mod support;

use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::Duration,
};
use support::{TempDir, commit_all, run_git};

const CONTRIBUTOR: &str = "river-contributor";
const MAINTAINER: &str = "maya-maintainer";
const REPOSITORY: &str = "dev/update-demo";

#[test]
fn two_actor_contribution_flow_agrees_across_cli_api_and_git() {
    if env::var_os("SCOPE_CLI_E2E").is_none() {
        return;
    }

    let api_url = env::var("SCOPE_API_URL").expect("SCOPE_API_URL is required for CLI E2E");
    let workspace = TempDir::new("contribution-flow");
    let contributor = Actor::new(CONTRIBUTOR, workspace.path(), &api_url);
    let maintainer = Actor::new(MAINTAINER, workspace.path(), &api_url);

    contributor.clone_repo(REPOSITORY);
    maintainer.clone_repo(REPOSITORY);
    assert!(
        !contributor.repo.join("internal/notes.md").exists(),
        "public contributor clone exposed a private file"
    );
    assert!(
        maintainer.repo.join("internal/notes.md").is_file(),
        "maintainer clone omitted a private file"
    );

    let started = contributor.json(["request", "start", "e2e-contribution"]);
    assert_command(&started, "request.start");
    let request_id = string_at(&started, "/result/request/id");
    assert_eq!(string_at(&started, "/result/request/state"), "Draft");

    let maintainer_drafts = maintainer.json(["request", "list"]);
    assert_command(&maintainer_drafts, "request.list");
    assert_request_absent(&maintainer_drafts, &request_id);

    fs::write(
        contributor.repo.join("contribution.txt"),
        "first public revision\n",
    )
    .unwrap();
    run_git(&contributor.repo, ["add", "contribution.txt"]);
    commit_all(&contributor.repo, "Add contribution flow fixture");
    let first_push = contributor.json(["request", "push"]);
    assert_command(&first_push, "request.push");
    let first_head = string_at(&first_push, "/result/request/head_oid");
    assert_eq!(string_at(&first_push, "/result/request/state"), "Draft");

    let submitted = contributor.json(["request", "submit", "--yes"]);
    assert_command(&submitted, "request.submit");
    assert_eq!(
        string_at(&submitted, "/result/response/request/state"),
        "Open"
    );
    let maintainer_open = maintainer.json(["request", "list"]);
    assert_request_state(&maintainer_open, &request_id, "Open");

    let edited = contributor.json(["request", "edit", "--title", "End-to-end contribution flow"]);
    assert_command(&edited, "request.edit");
    assert_eq!(
        string_at(&edited, "/result/response/request/title"),
        "End-to-end contribution flow"
    );
    let discussed = contributor.json([
        "request",
        "discuss",
        "--body",
        "Please review the second revision.",
    ]);
    assert_command(&discussed, "request.discuss");

    fs::write(
        contributor.repo.join("contribution.txt"),
        "second public revision\n",
    )
    .unwrap();
    run_git(&contributor.repo, ["add", "contribution.txt"]);
    commit_all(
        &contributor.repo,
        "Advance contribution without resubmitting",
    );
    let second_push = contributor.json(["request", "push"]);
    assert_command(&second_push, "request.push");
    let second_head = string_at(&second_push, "/result/request/head_oid");
    assert_ne!(first_head, second_head, "request head did not advance");
    assert_eq!(string_at(&second_push, "/result/request/state"), "Open");
    let maintainer_view = maintainer.json(["request", "show", "--request", request_id.as_str()]);
    assert_eq!(
        string_at(&maintainer_view, "/result/request/head_oid"),
        second_head
    );

    let merged = maintainer.json([
        "request",
        "merge",
        "--request",
        request_id.as_str(),
        "--yes",
    ]);
    assert_command(&merged, "request.merge");
    assert_eq!(
        string_at(&merged, "/result/response/request/state"),
        "Merged"
    );
    let contributor_rating = contributor.json([
        "request",
        "rate",
        "--request",
        request_id.as_str(),
        "--score",
        "5",
        "--reason",
        "Clear and timely review",
    ]);
    let maintainer_rating = maintainer.json([
        "request",
        "rate",
        "--request",
        request_id.as_str(),
        "--score",
        "5",
        "--reason",
        "Focused contribution",
    ]);
    assert_command(&contributor_rating, "request.rate");
    assert_command(&maintainer_rating, "request.rate");

    let close_started = contributor.json(["request", "start", "e2e-close"]);
    let close_id = string_at(&close_started, "/result/request/id");
    fs::write(contributor.repo.join("closed.txt"), "terminal request\n").unwrap();
    run_git(&contributor.repo, ["add", "closed.txt"]);
    commit_all(&contributor.repo, "Add close flow fixture");
    contributor.json(["request", "push"]);
    contributor.json(["request", "submit", "--yes"]);
    let closed = maintainer.json(["request", "close", "--request", close_id.as_str(), "--yes"]);
    assert_command(&closed, "request.close");
    assert_eq!(
        string_at(&closed, "/result/response/request/state"),
        "Closed"
    );
    let terminal_push = contributor.run(["--json", "request", "push"]);
    assert_eq!(terminal_push.status.code(), Some(4));
    assert!(terminal_push.stdout.is_empty());
    let terminal_error: Value = serde_json::from_slice(&terminal_push.stderr)
        .expect("terminal request failure must be JSON on stderr");
    assert_eq!(string_at(&terminal_error, "/code"), "forbidden");

    let public_checkout = workspace.path().join("public-after-merge");
    git_clone(
        &format!("{api_url}/git/public/dev/update-demo"),
        &public_checkout,
    );
    assert_eq!(
        fs::read_to_string(public_checkout.join("contribution.txt")).unwrap(),
        "second public revision\n"
    );
    assert!(!public_checkout.join("internal/notes.md").exists());

    let maintainer_after = maintainer.clone_to("maintainer-after-merge");
    assert!(maintainer_after.join("internal/notes.md").is_file());
    assert_eq!(
        fs::read_to_string(maintainer_after.join("contribution.txt")).unwrap(),
        "second public revision\n"
    );
}

struct Actor {
    handle: &'static str,
    api_url: String,
    config: PathBuf,
    repo: PathBuf,
}

impl Actor {
    fn new(handle: &'static str, workspace: &Path, api_url: &str) -> Self {
        let root = workspace.join(handle);
        let config = root.join("config");
        fs::create_dir_all(&config).unwrap();
        let response = reqwest::blocking::Client::new()
            .post(format!("{api_url}/v1/dev/cli-session/{handle}"))
            .send()
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<Value>()
            .unwrap();
        let session_token = string_at(&response, "/session_token");
        write_session(&config, api_url, &session_token);
        Self {
            handle,
            api_url: api_url.to_string(),
            config,
            repo: root.join("repo"),
        }
    }

    fn clone_repo(&self, repository: &str) {
        let output = self
            .command(self.repo.parent().unwrap())
            .args(["clone", repository, self.repo.to_str().unwrap()])
            .output()
            .unwrap();
        assert_success(&output, &format!("{} clone", self.handle));
    }

    fn clone_to(&self, name: &str) -> PathBuf {
        let destination = self.repo.parent().unwrap().join(name);
        let output = self
            .command(self.repo.parent().unwrap())
            .args(["clone", REPOSITORY, destination.to_str().unwrap()])
            .output()
            .unwrap();
        assert_success(&output, &format!("{} fresh clone", self.handle));
        destination
    }

    fn json<const N: usize>(&self, args: [&str; N]) -> Value {
        let action = args.join(" ");
        for attempt in 0..40 {
            let mut command_args = vec!["--json"];
            command_args.extend(args);
            let output = self.run(command_args);
            if output.status.success() {
                assert!(
                    output.stderr.is_empty(),
                    "{} scope {action} wrote stderr:\n{}",
                    self.handle,
                    String::from_utf8_lossy(&output.stderr)
                );
                return serde_json::from_slice(&output.stdout)
                    .expect("successful command must emit JSON");
            }
            let retryable = output.status.code() == Some(6)
                && serde_json::from_slice::<Value>(&output.stderr)
                    .ok()
                    .and_then(|error| error["retryable"].as_bool())
                    == Some(true);
            if retryable && attempt < 39 {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            assert_success(&output, &format!("{} scope {action}", self.handle));
        }
        unreachable!("bounded retry loop always returns or fails")
    }

    fn run<I, S>(&self, args: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.command(&self.repo).args(args).output().unwrap()
    }

    fn command(&self, cwd: &Path) -> Command {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_scope"));
        let binary_dir = binary.parent().unwrap();
        let path = env::join_paths(
            std::iter::once(binary_dir.to_path_buf())
                .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
        )
        .unwrap();
        let mut command = Command::new(binary);
        command
            .current_dir(cwd)
            .env("SCOPE_API_URL", &self.api_url)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("HOME", &self.config)
            .env("PATH", path);
        command
    }
}

fn write_session(config: &Path, api_url: &str, token: &str) {
    let key = api_url
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let sessions = config.join("scope/sessions");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(sessions.join(format!("cli-session-{key}")), token).unwrap();
}

fn git_clone(remote: &str, destination: &Path) {
    let output = Command::new("git")
        .args(["clone", remote, destination.to_str().unwrap()])
        .output()
        .unwrap();
    assert_success(&output, "public Git clone");
}

fn assert_command(document: &Value, expected: &str) {
    assert_eq!(
        document.pointer("/version").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(string_at(document, "/command"), expected);
}

fn assert_request_absent(document: &Value, request_id: &str) {
    let requests = document["result"]["requests"].as_array().unwrap();
    assert!(
        requests.iter().all(|request| request["id"] != request_id),
        "maintainer saw contributor draft {request_id}"
    );
}

fn assert_request_state(document: &Value, request_id: &str, state: &str) {
    let request = document["result"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|request| request["id"] == request_id)
        .unwrap_or_else(|| panic!("request {request_id} was not visible"));
    assert_eq!(request["state"], state);
}

fn string_at(document: &Value, pointer: &str) -> String {
    document
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string at {pointer}: {document}"))
        .to_string()
}

fn assert_success(output: &Output, action: &str) {
    assert!(
        output.status.success(),
        "{action} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
