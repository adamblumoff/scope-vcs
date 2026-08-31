use crate::{
    api::{
        AuthenticatedSession, BrowserLoginExchangeRequest, BrowserLoginStartRequest,
        BrowserLoginStartResponse, CLI_BROWSER_LOGIN_PATH, CLI_DEVICE_LOGIN_PATH,
        CLI_EXCHANGE_GRANTS_EXCHANGE_PATH, CliExchangeGrantExchangeRequest,
        CliSessionTokenResponse, DeviceLoginPollResponse, DeviceLoginStartResponse,
        DeviceLoginStatus, account_session, api_url, cli_browser_login_exchange_path,
        cli_device_login_poll_path, decode_json_response, display_user, http_client,
        revoke_cli_session, validate_session_token,
    },
    auth::{
        cached_cli_session, delete_stored_session_token, read_stored_session_token,
        store_session_token,
    },
    error::CliError,
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use std::{
    fs,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn login(
    headless: bool,
    exchange: Option<String>,
    exchange_file: Option<&Path>,
) -> anyhow::Result<()> {
    if headless && (exchange.is_some() || exchange_file.is_some()) {
        return Err(CliError::usage(
            "--headless cannot be used with --exchange or --exchange-file",
        )
        .into());
    }

    let api_url = api_url();
    let client = http_client()?;
    let exchange_token = match (exchange, exchange_file) {
        (Some(token), None) => Some(token),
        (None, Some(path)) => Some(read_private_exchange_token(path)?),
        (None, None) => None,
        (Some(_), Some(_)) => {
            return Err(
                CliError::usage("--exchange and --exchange-file cannot be used together").into(),
            );
        }
    };
    if let Some(exchange_token) = exchange_token {
        let session = exchange_login(&client, &api_url, &exchange_token)?;
        store_session_token(&api_url, &session.token)?;
        println!("Signed in as {}", display_user(&session.user));
        return Ok(());
    }

    let session = if headless {
        session_from_cache_or_login(&client, &api_url, |client, api_url| {
            device_login(client, api_url, false)
        })?
    } else {
        session_from_cache_or_browser(&client, &api_url)?
    };
    println!("Signed in as {}", display_user(&session.user));
    Ok(())
}

fn read_private_exchange_token(path: &Path) -> anyhow::Result<String> {
    let metadata = fs::symlink_metadata(path).context("inspect Scope exchange token file")?;
    if !metadata.file_type().is_file() {
        bail!("Scope exchange token path must be a regular file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            bail!("Scope exchange token file must not be accessible by group or other users");
        }
    }
    let token = fs::read_to_string(path).context("read Scope exchange token file")?;
    let token = token.trim();
    if token.is_empty() {
        bail!("Scope exchange token file is empty");
    }
    Ok(token.to_string())
}

pub fn logout() -> anyhow::Result<()> {
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

pub fn whoami() -> anyhow::Result<()> {
    let api_url = api_url();
    let client = http_client()?;
    let Some(session) = cached_cli_session(&client, &api_url)? else {
        return Err(CliError::authentication("not signed in; run scope login").into());
    };
    println!("{}", display_user(&session.user));
    let _ = account_session(&client, &api_url, &session.token)?;
    Ok(())
}

pub fn session_from_cache_or_browser(
    client: &Client,
    api_url: &str,
) -> anyhow::Result<AuthenticatedSession> {
    session_from_cache_or_login(client, api_url, local_browser_login)
}

pub fn session_from_cache_or_device(
    client: &Client,
    api_url: &str,
) -> anyhow::Result<AuthenticatedSession> {
    session_from_cache_or_login(client, api_url, |client, api_url| {
        device_login(client, api_url, false)
    })
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
    let response = client
        .post(format!("{api_url}{CLI_BROWSER_LOGIN_PATH}"))
        .json(&BrowserLoginStartRequest { callback_url })
        .send()
        .context("start browser login")?;
    let start: BrowserLoginStartResponse = decode_json_response(response, "start browser login")?;

    eprintln!("Opening browser to sign in:");
    eprintln!("{}", start.authorization_url);
    if let Err(error) = webbrowser::open(&start.authorization_url) {
        eprintln!("Could not open browser automatically: {error}");
        eprintln!("Open the URL above to continue.");
    }

    let callback_code =
        wait_for_browser_callback(&listener, &start.request_id, start.expires_at_unix)?;
    let response = client
        .post(format!(
            "{api_url}{}",
            cli_browser_login_exchange_path(&start.request_id)
        ))
        .json(&BrowserLoginExchangeRequest {
            request_secret: start.request_secret,
            callback_code,
        })
        .send()
        .context("exchange browser login")?;
    let exchanged: CliSessionTokenResponse =
        decode_json_response(response, "exchange browser login")?;
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
    let response = client
        .post(format!("{api_url}{CLI_EXCHANGE_GRANTS_EXCHANGE_PATH}"))
        .json(&CliExchangeGrantExchangeRequest {
            exchange_token: exchange_token.to_string(),
        })
        .send()
        .context("exchange Scope login token")?;
    let exchanged: CliSessionTokenResponse =
        decode_json_response(response, "exchange Scope login token")?;
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
    let response = client
        .post(format!("{api_url}{CLI_DEVICE_LOGIN_PATH}"))
        .send()
        .context("start browser login")?;
    let start: DeviceLoginStartResponse = decode_json_response(response, "start browser login")?;

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
        let response = client
            .post(format!(
                "{api_url}{}",
                cli_device_login_poll_path(&start.device_code)
            ))
            .send()
            .context("poll browser login")?;
        let poll: DeviceLoginPollResponse = decode_json_response(response, "poll browser login")?;

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
    if parts
        .next()
        .context("local Scope login callback missing method")?
        != "GET"
    {
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

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn exchange_token_file_must_be_private() {
        use crate::test_support::TestDir;
        use std::os::unix::fs::PermissionsExt;

        let temp = TestDir::new("exchange-token-file");
        let path = temp.path().join("exchange-token");
        fs::write(&path, "scope_otc_test\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            read_private_exchange_token(&path).unwrap(),
            "scope_otc_test"
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_private_exchange_token(&path).is_err());
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
