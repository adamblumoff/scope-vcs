use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use reqwest::blocking::Client;
use scope_cli::{
    api::{
        AuthenticatedSession, BrowserLoginExchangeRequest, BrowserLoginStartRequest,
        BrowserLoginStartResponse, CLI_BROWSER_LOGIN_PATH, CLI_DEVICE_LOGIN_PATH,
        CLI_EXCHANGE_GRANTS_EXCHANGE_PATH, CliExchangeGrantExchangeRequest,
        CliSessionTokenResponse, DeviceLoginPollResponse, DeviceLoginStartResponse,
        DeviceLoginStatus, RepoInitResponse, RepoPublicationState, api_url,
        cli_browser_login_exchange_path, cli_device_login_poll_path, create_push_intent,
        create_repo, display_user, get_repo, http_client, revoke_cli_session,
        rollback_created_repo, validate_session_token,
    },
    auth::{
        cached_cli_session, delete_stored_session_token, read_stored_session_token,
        store_session_token,
    },
    git_repo::{
        GitChangedPath, changed_paths_since_scope_base_at_commit, ensure_git_repo_ready,
        fetch_scope_remote_with_bearer, head_oid, mark_scope_remote_pushed, run_git,
        scope_remote_head_oid, warn_if_dirty_working_tree,
    },
    push::{
        DEFAULT_SCOPE_BRANCH, DEFAULT_SCOPE_REMOTE, ensure_scope_remote_can_receive_push,
        load_scope_remote, push_reviewed_head_with_intent,
    },
    repo_config::{
        config_visibility_label, ensure_scope_repo_config_exists,
        ensure_scope_repo_config_is_committed, load_scope_repo_config_at_commit, repo_config_path,
    },
};
use scope_core::domain::repo_config::{HistoryRewriteAction, RepoConfig};
use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

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
    Push(PushArgs),
    Clone(CloneArgs),
    Login(LoginArgs),
    Logout,
    Whoami,
}

#[derive(Parser)]
struct InitArgs {
    #[arg(long)]
    name: Option<String>,
}

#[derive(Parser)]
struct PushArgs {
    #[arg(long, default_value = DEFAULT_SCOPE_REMOTE)]
    remote: String,
    #[arg(short, long)]
    yes: bool,
}

#[derive(Parser)]
struct CloneArgs {
    repository: String,
    destination: Option<PathBuf>,
}

#[derive(Parser)]
struct LoginArgs {
    #[arg(long)]
    headless: bool,
    #[arg(long, value_name = "TOKEN")]
    exchange: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        CommandKind::Init(args) => init(args),
        CommandKind::Push(args) => push(args),
        CommandKind::Clone(args) => clone(args),
        CommandKind::Login(args) => login(args),
        CommandKind::Logout => logout(),
        CommandKind::Whoami => whoami(),
    }
}

fn init(args: InitArgs) -> anyhow::Result<()> {
    let git_repo = ensure_git_repo_ready("scope init")?;
    let api_url = api_url();
    let repo_name = match args.name.as_deref() {
        Some(name) => normalize_repo_name(name)?,
        None => prompt_repo_name(&git_repo.root)?,
    };
    warn_if_dirty_working_tree(&git_repo)?;

    let client = http_client()?;
    let session = session_from_cache_or_login(&client, &api_url, local_browser_login)?;
    eprintln!("Signed in as {}", display_user(&session.user));
    let created = create_repo(&client, &api_url, &session.token, repo_name)?;

    let config_created = match configure_remote(&created.init)
        .and_then(|_| ensure_scope_repo_config_exists(&git_repo.root))
    {
        Ok(config_created) => config_created,
        Err(error) => {
            rollback_created_repo(&client, &api_url, &session.token, &created.repo);
            return Err(error);
        }
    };

    println!(
        "Created Scope repo: {}/{}",
        created.repo.owner_handle, created.repo.name
    );
    println!("Configured Git remote: {}", created.init.remote_name);
    if config_created {
        println!("Created {}", repo_config_path());
        println!("Commit the config file, then run: scope push");
    } else {
        println!("Using existing {}", repo_config_path());
        println!("Run: scope push");
    }

    Ok(())
}

fn push(args: PushArgs) -> anyhow::Result<()> {
    let git_repo = ensure_git_repo_ready("scope push")?;
    let reviewed_head_oid = head_oid(&git_repo)?;
    warn_if_dirty_working_tree(&git_repo)?;
    ensure_scope_repo_config_is_committed(&git_repo.root)?;
    let config = load_scope_repo_config_at_commit(&git_repo.root, &reviewed_head_oid)?;

    let api_url = api_url();
    let target = load_scope_remote(&api_url, &args.remote)?;
    let client = http_client()?;
    let session = session_from_cache_or_login(&client, &api_url, local_browser_login)?;

    let repo = get_repo(
        &client,
        &api_url,
        &session.token,
        &target.owner,
        &target.repo,
    )?;
    ensure_scope_remote_can_receive_push(&target, &repo)?;
    if repo.lifecycle_state == RepoPublicationState::Published {
        fetch_scope_remote_with_bearer(
            &git_repo,
            &target.push_url,
            &args.remote,
            DEFAULT_SCOPE_BRANCH,
            &session.token,
        )?;
    }

    let intent = create_push_intent(
        &client,
        &api_url,
        &session.token,
        &target.owner,
        &target.repo,
        &reviewed_head_oid,
    )?;
    ensure_review_base_matches_intent(
        &git_repo,
        &target.push_url,
        &args.remote,
        &session.token,
        intent.base_head_oid.as_deref(),
    )?;
    let changed_paths = changed_paths_since_scope_base_at_commit(
        &git_repo,
        intent.base_head_oid.as_deref(),
        &reviewed_head_oid,
    )?;
    confirm_scope_push(&args, &config, &changed_paths)?;
    ensure_push_intent_not_expired(intent.expires_at_unix)?;

    let outcome = match push_reviewed_head_with_intent(
        &client,
        &api_url,
        &session.token,
        &target,
        &reviewed_head_oid,
        &intent.token,
    ) {
        Ok(outcome) => outcome,
        Err(_error) if push_intent_expired(intent.expires_at_unix) => {
            bail!("Scope push review expired; rerun scope push");
        }
        Err(error) => return Err(error),
    };
    mark_scope_remote_pushed(
        &git_repo,
        &args.remote,
        DEFAULT_SCOPE_BRANCH,
        &reviewed_head_oid,
    )?;

    if outcome.staged_update_pending {
        println!(
            "Pushed to Scope: {}/{}\nScope reported a pending update; this should not happen with config-owned pushes.",
            outcome.owner, outcome.repo
        );
    } else {
        println!(
            "Pushed to Scope: {}/{}\nPush applied by Scope.",
            outcome.owner, outcome.repo
        );
    }

    Ok(())
}

fn ensure_push_intent_not_expired(expires_at_unix: u64) -> anyhow::Result<()> {
    if !push_intent_expired(expires_at_unix) {
        return Ok(());
    }

    bail!("Scope push review expired; rerun scope push")
}

fn push_intent_expired(expires_at_unix: u64) -> bool {
    unix_now() >= expires_at_unix
}

fn ensure_review_base_matches_intent(
    git_repo: &scope_cli::git_repo::GitRepo,
    push_url: &str,
    remote: &str,
    session_token: &str,
    intent_base_head_oid: Option<&str>,
) -> anyhow::Result<()> {
    let Some(intent_base_head_oid) = intent_base_head_oid else {
        return Ok(());
    };
    if scope_remote_head_oid(git_repo, remote, DEFAULT_SCOPE_BRANCH)?.as_deref()
        == Some(intent_base_head_oid)
    {
        return Ok(());
    }

    fetch_scope_remote_with_bearer(
        git_repo,
        push_url,
        remote,
        DEFAULT_SCOPE_BRANCH,
        session_token,
    )?;
    if scope_remote_head_oid(git_repo, remote, DEFAULT_SCOPE_BRANCH)?.as_deref()
        == Some(intent_base_head_oid)
    {
        return Ok(());
    }

    bail!("Scope changed while preparing push review; rerun scope push");
}

fn confirm_scope_push(
    args: &PushArgs,
    config: &RepoConfig,
    changed_paths: &[GitChangedPath],
) -> anyhow::Result<()> {
    print_scope_push_review(config, changed_paths);
    if args.yes {
        return Ok(());
    }

    eprint!("Apply this Scope push? [y/N]: ");
    io::stderr().flush().ok();
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("read Scope push confirmation")?;
    match input.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(()),
        _ => bail!("scope push cancelled"),
    }
}

fn print_scope_push_review(config: &RepoConfig, changed_paths: &[GitChangedPath]) {
    eprintln!("Scope push review");
    eprintln!("Config: {}", repo_config_path());
    eprintln!(
        "Default visibility: {}",
        config_visibility_label(config.visibility.default_visibility())
    );
    if config.visibility.rules.is_empty() {
        eprintln!("Visibility rules: none");
    } else {
        eprintln!("Visibility rules:");
        for rule in &config.visibility.rules {
            eprintln!(
                "  {} -> {}",
                rule.path,
                config_visibility_label(rule.visibility)
            );
        }
    }

    if config.history.rewrites.is_empty() {
        eprintln!("History rewrites: none");
    } else {
        eprintln!("History rewrites:");
        for rewrite in &config.history.rewrites {
            let action = match rewrite.action {
                HistoryRewriteAction::RedactPublicHistory => "redact public history",
            };
            eprintln!("  {} -> {}", rewrite.path, action);
        }
    }

    if changed_paths.is_empty() {
        eprintln!("Committed file changes since last Scope push: none");
    } else {
        eprintln!("Committed file changes since last Scope push:");
        for change in changed_paths {
            eprintln!("  {} {}", change.status, change.path);
        }
    }
}

fn clone(args: CloneArgs) -> anyhow::Result<()> {
    scope_cli::clone::clone_repo(&args.repository, args.destination.as_deref())
}

fn login(args: LoginArgs) -> anyhow::Result<()> {
    if args.headless && args.exchange.is_some() {
        bail!("--headless and --exchange cannot be used together");
    }

    let api_url = api_url();
    let client = http_client()?;

    if let Some(exchange_token) = args.exchange {
        let session = exchange_login(&client, &api_url, &exchange_token)?;
        store_session_token(&api_url, &session.token)?;
        println!("Signed in as {}", display_user(&session.user));
        return Ok(());
    }

    let session = if args.headless {
        session_from_cache_or_login(&client, &api_url, |client, api_url| {
            device_login(client, api_url, false)
        })?
    } else {
        session_from_cache_or_login(&client, &api_url, local_browser_login)?
    };
    println!("Signed in as {}", display_user(&session.user));
    Ok(())
}

fn logout() -> anyhow::Result<()> {
    let api_url = api_url();
    let client = http_client()?;

    let Some(token) = read_stored_session_token(&api_url)? else {
        println!("Not signed in");
        return Ok(());
    };

    if let Err(error) = revoke_cli_session(&client, &api_url, &token) {
        eprintln!("Could not revoke the server session: {error}");
    }
    delete_stored_session_token(&api_url)?;
    println!("Signed out");
    Ok(())
}

fn whoami() -> anyhow::Result<()> {
    let api_url = api_url();
    let client = http_client()?;

    let Some(session) = cached_cli_session(&client, &api_url)? else {
        bail!("not signed in; run scope login");
    };
    println!("{}", display_user(&session.user));
    Ok(())
}

fn session_from_cache_or_login(
    client: &Client,
    api_url: &str,
    login: impl FnOnce(&Client, &str) -> anyhow::Result<AuthenticatedSession>,
) -> anyhow::Result<AuthenticatedSession> {
    if let Some(session) = cached_cli_session(client, api_url)? {
        return Ok(session);
    }

    let session = login(client, api_url)?;
    store_session_token(api_url, &session.token)?;
    Ok(session)
}

fn local_browser_login(client: &Client, api_url: &str) -> anyhow::Result<AuthenticatedSession> {
    let listener = TcpListener::bind("127.0.0.1:0").context("bind local Scope login callback")?;
    listener
        .set_nonblocking(true)
        .context("configure local Scope login callback")?;
    let port = listener
        .local_addr()
        .context("read local Scope login callback address")?
        .port();
    let callback_url = format!("http://127.0.0.1:{port}/scope-cli-callback");
    let start: BrowserLoginStartResponse = client
        .post(format!("{api_url}{CLI_BROWSER_LOGIN_PATH}"))
        .json(&BrowserLoginStartRequest { callback_url })
        .send()
        .context("start browser login")?
        .error_for_status()
        .context("start browser login")?
        .json()
        .context("parse browser login response")?;

    eprintln!("Opening browser to sign in:");
    eprintln!("{}", start.authorization_url);
    if let Err(error) = webbrowser::open(&start.authorization_url) {
        eprintln!("Could not open browser automatically: {error}");
        eprintln!("Open the URL above to continue.");
    }

    let callback_code =
        wait_for_browser_callback(&listener, &start.request_id, start.expires_at_unix)?;
    let exchanged: CliSessionTokenResponse = client
        .post(format!(
            "{api_url}{}",
            cli_browser_login_exchange_path(&start.request_id)
        ))
        .json(&BrowserLoginExchangeRequest {
            request_secret: start.request_secret,
            callback_code,
        })
        .send()
        .context("exchange browser login")?
        .error_for_status()
        .context("exchange browser login")?
        .json()
        .context("parse browser login exchange response")?;
    let user = validate_session_token(client, api_url, &exchanged.session_token)?
        .context("completed login did not create a valid CLI session")?;
    Ok(AuthenticatedSession {
        token: exchanged.session_token,
        user,
    })
}

fn exchange_login(
    client: &Client,
    api_url: &str,
    exchange_token: &str,
) -> anyhow::Result<AuthenticatedSession> {
    let exchanged: CliSessionTokenResponse = client
        .post(format!("{api_url}{CLI_EXCHANGE_GRANTS_EXCHANGE_PATH}"))
        .json(&CliExchangeGrantExchangeRequest {
            exchange_token: exchange_token.to_string(),
        })
        .send()
        .context("exchange Scope login token")?
        .error_for_status()
        .context("exchange Scope login token")?
        .json()
        .context("parse Scope login exchange response")?;
    let user = validate_session_token(client, api_url, &exchanged.session_token)?
        .context("exchange token did not create a valid CLI session")?;
    Ok(AuthenticatedSession {
        token: exchanged.session_token,
        user,
    })
}

fn device_login(
    client: &Client,
    api_url: &str,
    open_browser: bool,
) -> anyhow::Result<AuthenticatedSession> {
    let start: DeviceLoginStartResponse = client
        .post(format!("{api_url}{CLI_DEVICE_LOGIN_PATH}"))
        .send()
        .context("start browser login")?
        .error_for_status()
        .context("start browser login")?
        .json()
        .context("parse browser login response")?;

    eprintln!("Open this URL to sign in:");
    eprintln!("{}", start.verification_url);
    eprintln!("Code: {}", format_user_code(&start.user_code));
    if open_browser && let Err(error) = webbrowser::open(&start.verification_url) {
        eprintln!("Could not open browser automatically: {error}");
    }

    loop {
        if unix_now() >= start.expires_at_unix {
            bail!("browser login expired");
        }

        thread::sleep(Duration::from_secs(start.poll_interval_secs.max(1)));
        let poll: DeviceLoginPollResponse = client
            .post(format!(
                "{api_url}{}",
                cli_device_login_poll_path(&start.device_code)
            ))
            .send()
            .context("poll browser login")?
            .error_for_status()
            .context("poll browser login")?
            .json()
            .context("parse browser login poll response")?;

        if matches!(poll.status, DeviceLoginStatus::Complete) {
            let token = poll
                .session_token
                .context("completed login missing token")?;
            let user = validate_session_token(client, api_url, &token)?
                .context("completed login did not create a valid CLI session")?;
            return Ok(AuthenticatedSession { token, user });
        }
    }
}

fn wait_for_browser_callback(
    listener: &TcpListener,
    expected_request_id: &str,
    expires_at_unix: u64,
) -> anyhow::Result<String> {
    eprintln!("Waiting for browser confirmation...");
    loop {
        if unix_now() >= expires_at_unix {
            bail!("browser login expired");
        }

        match listener.accept() {
            Ok((mut stream, _)) => match read_browser_callback(&mut stream, expected_request_id) {
                Ok(callback_code) => {
                    write_browser_callback_response(
                        &mut stream,
                        "200 OK",
                        "Scope CLI sign-in complete. You can close this tab.",
                    );
                    return Ok(callback_code);
                }
                Err(error) => {
                    write_browser_callback_response(
                        &mut stream,
                        "400 Bad Request",
                        "Scope CLI sign-in failed. Return to your terminal and try again.",
                    );
                    eprintln!("Ignoring invalid browser callback: {error}");
                }
            },
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error).context("accept local Scope login callback"),
        }
    }
}

fn read_browser_callback(
    stream: &mut TcpStream,
    expected_request_id: &str,
) -> anyhow::Result<String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .context("configure local Scope login callback timeout")?;
    let request_line = read_http_request_line(stream)?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .context("local Scope login callback missing method")?;
    if method != "GET" {
        bail!("local Scope login callback must use GET");
    }
    let target = parts
        .next()
        .context("local Scope login callback missing path")?;
    browser_callback_code_from_target(target, expected_request_id)
}

fn read_http_request_line(stream: &mut TcpStream) -> anyhow::Result<String> {
    let mut request = Vec::with_capacity(512);
    let mut chunk = [0_u8; 256];
    loop {
        if request.len() > 4096 {
            bail!("local Scope login callback request line is too large");
        }
        if let Some(line_end) = request.iter().position(|byte| *byte == b'\n') {
            let line = String::from_utf8_lossy(&request[..line_end])
                .trim_end_matches('\r')
                .to_string();
            if line.is_empty() {
                bail!("local Scope login callback was empty");
            }
            return Ok(line);
        }

        match stream.read(&mut chunk) {
            Ok(0) => bail!("local Scope login callback closed before request line"),
            Ok(byte_count) => request.extend_from_slice(&chunk[..byte_count]),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                bail!("local Scope login callback timed out")
            }
            Err(error) => return Err(error).context("read local Scope login callback"),
        }
    }
}

fn browser_callback_code_from_target(
    target: &str,
    expected_request_id: &str,
) -> anyhow::Result<String> {
    let url = if target.starts_with("http://") {
        reqwest::Url::parse(target)
    } else {
        reqwest::Url::parse(&format!("http://127.0.0.1{target}"))
    }
    .context("parse local Scope login callback URL")?;
    if url.path() != "/scope-cli-callback" {
        bail!("local Scope login callback used an unexpected path");
    }

    let mut request_id = None;
    let mut callback_code = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "request_id" => request_id = Some(value.into_owned()),
            "code" => callback_code = Some(value.into_owned()),
            _ => {}
        }
    }

    if request_id.as_deref() != Some(expected_request_id) {
        bail!("local Scope login callback request id did not match");
    }
    callback_code.context("local Scope login callback missing code")
}

fn write_browser_callback_response(stream: &mut TcpStream, status: &str, message: &str) {
    let body = format!("<!doctype html><title>Scope CLI</title><p>{message}</p>");
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn format_user_code(code: &str) -> String {
    code.as_bytes()
        .chunks(4)
        .map(|chunk| String::from_utf8_lossy(chunk).to_string())
        .collect::<Vec<_>>()
        .join("-")
}

fn configure_remote(init: &RepoInitResponse) -> anyhow::Result<()> {
    let _ = Command::new("git")
        .args(["remote", "remove", &init.remote_name])
        .status();
    run_git(&["remote", "add", &init.remote_name, &init.git_remote_url])
}

fn prompt_repo_name(git_root: &Path) -> anyhow::Result<String> {
    let default = default_repo_name(git_root);
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

fn default_repo_name(git_root: &Path) -> String {
    git_root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "repo".to_string())
}

fn normalize_repo_name(name: &str) -> anyhow::Result<String> {
    let name = name.trim().to_ascii_lowercase();
    if name.is_empty() {
        bail!("repository name is required");
    }
    Ok(name)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_code_is_grouped_for_manual_entry() {
        assert_eq!(format_user_code("ABCDEF1234567890"), "ABCD-EF12-3456-7890");
    }

    #[test]
    fn expired_push_intent_reports_rerun_message() {
        let error = ensure_push_intent_not_expired(0).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Scope push review expired; rerun scope push")
        );
        ensure_push_intent_not_expired(unix_now().saturating_add(60)).unwrap();
    }

    #[test]
    fn browser_callback_target_returns_code_for_matching_request() {
        assert_eq!(
            browser_callback_code_from_target(
                "/scope-cli-callback?request_id=cli_browser_123&code=scope_callback_456",
                "cli_browser_123"
            )
            .unwrap(),
            "scope_callback_456"
        );
    }

    #[test]
    fn browser_callback_target_rejects_mismatched_request() {
        assert!(
            browser_callback_code_from_target(
                "/scope-cli-callback?request_id=cli_browser_other&code=scope_callback_456",
                "cli_browser_123"
            )
            .is_err()
        );
    }

    #[test]
    fn browser_callback_reader_accepts_split_request_line() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let writer = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .write_all(b"GET /scope-cli-callback?request_id=cli_browser_123")
                .unwrap();
            thread::sleep(Duration::from_millis(25));
            stream
                .write_all(b"&code=scope_callback_456 HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .unwrap();
        });

        let (mut stream, _) = listener.accept().unwrap();
        assert_eq!(
            read_browser_callback(&mut stream, "cli_browser_123").unwrap(),
            "scope_callback_456"
        );
        writer.join().unwrap();
    }

    #[test]
    fn default_repo_name_uses_git_root_folder_name() {
        assert_eq!(
            default_repo_name(Path::new("C:/Users/adam/Code/scope-vcs")),
            "scope-vcs"
        );
    }
}
