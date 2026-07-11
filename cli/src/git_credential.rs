use crate::auth::read_stored_session_token;
use anyhow::Context;
use reqwest::Url;
use std::io::{self, BufRead, Write};

#[derive(Debug, Default, Eq, PartialEq)]
struct GitCredentialRequest {
    protocol: Option<String>,
    host: Option<String>,
    path: Option<String>,
}

pub fn run_git_credential(operation: &str) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    write_git_credential_response(operation, stdin.lock(), stdout.lock())
}

fn write_git_credential_response(
    operation: &str,
    reader: impl BufRead,
    writer: impl Write,
) -> anyhow::Result<()> {
    write_git_credential_response_with(operation, reader, writer, read_stored_session_token)
}

fn write_git_credential_response_with(
    operation: &str,
    reader: impl BufRead,
    mut writer: impl Write,
    read_token: impl FnOnce(&str) -> anyhow::Result<Option<String>>,
) -> anyhow::Result<()> {
    let request = parse_git_credential_request(reader)?;
    if operation != "get" {
        return Ok(());
    }

    let Some(api_url) = scope_api_url_for_credential_request(&request) else {
        return Ok(());
    };
    let Some(session_token) = read_token(&api_url)? else {
        return Ok(());
    };

    writeln!(writer, "username=scope").context("write Git credential username")?;
    writeln!(writer, "password={session_token}").context("write Git credential password")?;
    writeln!(writer).context("finish Git credential response")?;
    Ok(())
}

fn parse_git_credential_request(reader: impl BufRead) -> anyhow::Result<GitCredentialRequest> {
    let mut request = GitCredentialRequest::default();
    for line in reader.lines() {
        let line = line.context("read Git credential request")?;
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "protocol" => request.protocol = Some(value.to_string()),
            "host" => request.host = Some(value.to_string()),
            "path" => request.path = Some(value.to_string()),
            _ => {}
        }
    }
    Ok(request)
}

fn scope_api_url_for_credential_request(request: &GitCredentialRequest) -> Option<String> {
    let protocol = request.protocol.as_deref()?;
    if protocol != "http" && protocol != "https" {
        return None;
    }
    let host = request.host.as_deref()?.trim();
    if host.is_empty() {
        return None;
    }
    let path = request.path.as_deref()?;
    let path = format!("/{}", path.trim_start_matches('/'));
    let marker = "/git/permissioned/";
    let marker_start = path.find(marker)?;
    let repo_path = &path[marker_start + marker.len()..];
    let mut repo_segments = repo_path.split('/').filter(|segment| !segment.is_empty());
    repo_segments.next()?;
    repo_segments.next()?;

    let api_path = path[..marker_start].trim_end_matches('/');
    let api_url = format!("{protocol}://{host}{api_path}");
    let parsed = Url::parse(&api_url).ok()?;
    Some(parsed.as_str().trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parse_git_credential_request_reads_scope_fields() {
        let request = parse_git_credential_request(Cursor::new(
            "protocol=https\nhost=scope.example\npath=git/permissioned/adam/repo\n\n",
        ))
        .unwrap();

        assert_eq!(
            request,
            GitCredentialRequest {
                protocol: Some("https".to_string()),
                host: Some("scope.example".to_string()),
                path: Some("git/permissioned/adam/repo".to_string()),
            }
        );
    }

    #[test]
    fn scope_api_url_for_credential_request_derives_scope_api_url() {
        let request = GitCredentialRequest {
            protocol: Some("https".to_string()),
            host: Some("scope.example:8443".to_string()),
            path: Some("api/git/permissioned/adam/repo".to_string()),
        };

        assert_eq!(
            scope_api_url_for_credential_request(&request),
            Some("https://scope.example:8443/api".to_string())
        );
    }

    #[test]
    fn scope_api_url_for_credential_request_ignores_public_or_incomplete_paths() {
        for path in [
            "git/public/adam/repo",
            "git/permissioned/adam",
            "other/permissioned/adam/repo",
        ] {
            let request = GitCredentialRequest {
                protocol: Some("https".to_string()),
                host: Some("scope.example".to_string()),
                path: Some(path.to_string()),
            };
            assert_eq!(scope_api_url_for_credential_request(&request), None);
        }
    }

    #[test]
    fn write_git_credential_response_ignores_non_get_operations() {
        let mut output = Vec::new();
        write_git_credential_response(
            "store",
            Cursor::new("protocol=https\nhost=scope.example\npath=git/permissioned/adam/repo\n\n"),
            &mut output,
        )
        .unwrap();

        assert!(output.is_empty());
    }

    #[test]
    fn get_returns_the_session_for_the_derived_api_url() {
        let mut output = Vec::new();
        write_git_credential_response_with(
            "get",
            Cursor::new(
                "protocol=https\nhost=scope.example\npath=api/git/permissioned/adam/repo\n\n",
            ),
            &mut output,
            |api_url| {
                assert_eq!(api_url, "https://scope.example/api");
                Ok(Some("scope_cli_secret".to_string()))
            },
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "username=scope\npassword=scope_cli_secret\n\n"
        );
    }
}
