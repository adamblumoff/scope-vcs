use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{
    env,
    io::{self, Write},
    process::Command,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DEFAULT_API_URL: &str = "https://scope-api-production-0251.up.railway.app";

#[derive(Parser)]
#[command(name = "scope")]
#[command(about = "Scope VCS command line")]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
    Init(InitArgs),
}

#[derive(Parser)]
struct InitArgs {
    name: Option<String>,
    #[arg(long)]
    public: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
enum Visibility {
    Private,
    Public,
}

#[derive(Deserialize)]
struct DeviceLoginStartResponse {
    device_code: String,
    verification_url: String,
    expires_at_unix: u64,
    poll_interval_secs: u64,
}

#[derive(Deserialize)]
struct DeviceLoginPollResponse {
    status: DeviceLoginStatus,
    access_token: Option<String>,
}

#[derive(Deserialize)]
enum DeviceLoginStatus {
    Pending,
    Complete,
}

#[derive(Serialize)]
struct CreateRepoRequest {
    name: String,
    visibility: Visibility,
}

#[derive(Deserialize)]
struct CreateRepoResponse {
    repo: RepoSummaryResponse,
    init: RepoInitResponse,
}

#[derive(Deserialize)]
struct RepoSummaryResponse {
    owner_handle: String,
    name: String,
}

#[derive(Deserialize)]
struct RepoInitResponse {
    git_remote_url: String,
    remote_name: String,
    push_branch: String,
    push_token: Option<GitPushTokenResponse>,
    review_url: String,
}

#[derive(Deserialize)]
struct GitPushTokenResponse {
    secret: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        CommandKind::Init(args) => init(args),
    }
}

fn init(args: InitArgs) -> anyhow::Result<()> {
    let api_url = api_url();
    let repo_name = match args.name {
        Some(name) => normalize_repo_name(&name)?,
        None => prompt_repo_name()?,
    };
    let visibility = if args.public {
        Visibility::Public
    } else {
        Visibility::Private
    };

    ensure_git_repo_ready()?;

    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("build HTTP client")?;
    let access_token = browser_login(&client, &api_url)?;
    let created = create_repo(&client, &api_url, &access_token, repo_name, visibility)?;
    let push_secret = created
        .init
        .push_token
        .as_ref()
        .and_then(|token| token.secret.as_ref())
        .cloned()
        .context("API did not return a Git push token")?;

    if let Err(error) = configure_remote(&created.init)
        .and_then(|_| push_initial_commit(&created.init, &push_secret))
    {
        rollback_created_repo(&client, &api_url, &access_token, &created.repo);
        return Err(error);
    }

    println!("{}", created.init.review_url);
    Ok(())
}

fn browser_login(client: &Client, api_url: &str) -> anyhow::Result<String> {
    let start: DeviceLoginStartResponse = client
        .post(format!("{api_url}/v1/cli/device-login"))
        .send()
        .context("start browser login")?
        .error_for_status()
        .context("start browser login")?
        .json()
        .context("parse browser login response")?;

    eprintln!("{}", start.verification_url);
    let _ = webbrowser::open(&start.verification_url);

    loop {
        if unix_now() >= start.expires_at_unix {
            bail!("browser login expired");
        }

        thread::sleep(Duration::from_secs(start.poll_interval_secs.max(1)));
        let poll: DeviceLoginPollResponse = client
            .post(format!(
                "{api_url}/v1/cli/device-login/{}/poll",
                start.device_code
            ))
            .send()
            .context("poll browser login")?
            .error_for_status()
            .context("poll browser login")?
            .json()
            .context("parse browser login poll response")?;

        if matches!(poll.status, DeviceLoginStatus::Complete) {
            return poll.access_token.context("completed login missing token");
        }
    }
}

fn create_repo(
    client: &Client,
    api_url: &str,
    access_token: &str,
    name: String,
    visibility: Visibility,
) -> anyhow::Result<CreateRepoResponse> {
    client
        .post(format!("{api_url}/v1/repos"))
        .bearer_auth(access_token)
        .json(&CreateRepoRequest { name, visibility })
        .send()
        .context("create Scope repository")?
        .error_for_status()
        .context("create Scope repository")?
        .json()
        .context("parse create repository response")
}

fn rollback_created_repo(
    client: &Client,
    api_url: &str,
    access_token: &str,
    repo: &RepoSummaryResponse,
) {
    let result = client
        .delete(format!(
            "{api_url}/v1/repos/{}/{}",
            repo.owner_handle, repo.name
        ))
        .bearer_auth(access_token)
        .send();

    match result {
        Ok(response) if response.status().is_success() => {
            eprintln!("Deleted Scope repository after failed init");
        }
        Ok(response) => {
            eprintln!(
                "Scope repository was created, but rollback failed: {}",
                response.status()
            );
        }
        Err(error) => {
            eprintln!("Scope repository was created, but rollback failed: {error}");
        }
    }
}

fn configure_remote(init: &RepoInitResponse) -> anyhow::Result<()> {
    let _ = Command::new("git")
        .args(["remote", "remove", &init.remote_name])
        .status();
    run_git(&["remote", "add", &init.remote_name, &init.git_remote_url])
}

fn push_initial_commit(init: &RepoInitResponse, push_secret: &str) -> anyhow::Result<()> {
    let auth_header = format!("http.extraHeader=Authorization: Bearer {push_secret}");
    run_git(&[
        "-c",
        &auth_header,
        "push",
        &init.remote_name,
        &format!("HEAD:{}", init.push_branch),
    ])
}

fn ensure_git_repo_ready() -> anyhow::Result<()> {
    if !git_success(&["rev-parse", "--is-inside-work-tree"]) {
        eprintln!("Initializing Git repository");
        run_git(&["init", "-b", "main"])?;
    }

    if !git_success(&["rev-parse", "--verify", "HEAD"]) {
        bail!("create at least one Git commit before running scope init");
    }

    Ok(())
}

fn run_git(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("git")
        .args(args)
        .status()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !status.success() {
        bail!("git {} failed", args.join(" "));
    }
    Ok(())
}

fn git_success(args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

fn prompt_repo_name() -> anyhow::Result<String> {
    let default = env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "repo".to_string());
    eprint!("Repository name [{default}]: ");
    io::stderr().flush().ok();

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("read repository name")?;
    let name = if input.trim().is_empty() {
        default
    } else {
        input
    };
    normalize_repo_name(&name)
}

fn normalize_repo_name(name: &str) -> anyhow::Result<String> {
    let name = name.trim().to_ascii_lowercase();
    if name.is_empty() {
        bail!("repository name is required");
    }
    Ok(name)
}

fn api_url() -> String {
    env::var("SCOPE_API_URL")
        .or_else(|_| env::var("SCOPE_API_PUBLIC_URL"))
        .ok()
        .or_else(|| option_env!("SCOPE_API_URL").map(str::to_string))
        .or_else(|| option_env!("SCOPE_API_PUBLIC_URL").map(str::to_string))
        .unwrap_or_else(|| DEFAULT_API_URL.to_string())
        .trim_end_matches('/')
        .to_string()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
