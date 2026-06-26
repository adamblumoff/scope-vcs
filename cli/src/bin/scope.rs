use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use reqwest::blocking::Client;
use scope_cli::{
    api::{
        AuthenticatedSession, BrowserLoginExchangeRequest, BrowserLoginStartRequest,
        BrowserLoginStartResponse, CliExchangeGrantExchangeRequest, CliSessionTokenResponse,
        DeviceLoginPollResponse, DeviceLoginStartResponse, DeviceLoginStatus, RepoInitResponse,
        Visibility, api_url, create_repo, display_user, http_client, revoke_cli_session,
        rollback_created_repo, validate_session_token,
    },
    auth::{
        cached_cli_session, delete_stored_session_token, ensure_cli_session,
        read_stored_session_token, store_session_token,
    },
};
use std::{
    env,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
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
    Login(LoginArgs),
    Logout,
    Whoami,
}

#[derive(Parser)]
struct InitArgs {
    name: Option<String>,
    #[arg(long)]
    public: bool,
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
        CommandKind::Login(args) => login(args),
        CommandKind::Logout => logout(),
        CommandKind::Whoami => whoami(),
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

    let client = http_client()?;
    let session = ensure_cli_session(&client, &api_url)?;
    let created = create_repo(&client, &api_url, &session.token, repo_name, visibility)?;
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
        rollback_created_repo(&client, &api_url, &session.token, &created.repo);
        return Err(error);
    }

    println!("{}", created.init.review_url);
    Ok(())
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

    if let Some(session) = cached_cli_session(&client, &api_url)? {
        println!("Signed in as {}", display_user(&session.user));
        return Ok(());
    }

    let session = if args.headless {
        device_login(&client, &api_url, false)?
    } else {
        local_browser_login(&client, &api_url)?
    };
    store_session_token(&api_url, &session.token)?;
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
        .post(format!("{api_url}/v1/cli/browser-login"))
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
            "{api_url}/v1/cli/browser-login/{}/exchange",
            start.request_id
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
        .post(format!("{api_url}/v1/cli/exchange-grants/exchange"))
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
        .post(format!("{api_url}/v1/cli/device-login"))
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
}
