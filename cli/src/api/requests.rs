use super::*;
use anyhow::Context;
use reqwest::blocking::{Client, RequestBuilder};
use serde::de::DeserializeOwned;

#[derive(Clone, Copy)]
pub struct RequestTarget<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub request_id: &'a str,
}

pub struct StartRequestParams<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub name: String,
    pub title: Option<String>,
    pub audience: RequestAudience,
}

pub struct CreateRequestDiscussionParams<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub request_id: &'a str,
    pub body_markdown: String,
    pub client_discussion_id: String,
}

pub struct RequestActivityParams<'a> {
    pub target: RequestTarget<'a>,
    pub after: Option<u64>,
    pub latest: bool,
    pub limit: Option<usize>,
}

pub fn list_requests(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    cursor: Option<&str>,
) -> anyhow::Result<RequestListResponse> {
    let mut request = client
        .get(format!(
            "{api_url}{}",
            scope_api_contract::routes::repo_requests(owner, repo)
        ))
        .bearer_auth(session_token);
    if let Some(cursor) = cursor {
        request = request.query(&[("cursor", cursor)]);
    }
    execute_repo_request(request, owner, repo, "list requests")
}

pub fn get_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    request_id: &str,
) -> anyhow::Result<RequestDetailResponse> {
    let target = RequestTarget {
        owner,
        repo,
        request_id,
    };
    execute_request(
        client
            .get(request_url(api_url, target))
            .bearer_auth(session_token),
        target,
        "load request",
    )
}

pub fn close_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    request_id: &str,
) -> anyhow::Result<RequestCloseResponse> {
    let target = RequestTarget {
        owner,
        repo,
        request_id,
    };
    execute_request(
        client
            .delete(request_url(api_url, target))
            .bearer_auth(session_token),
        target,
        "close request",
    )
}

pub fn start_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    params: StartRequestParams<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    let owner = params.owner;
    let repo = params.repo;
    execute_repo_request(
        client
            .post(format!(
                "{api_url}{}",
                scope_api_contract::routes::repo_requests(owner, repo)
            ))
            .bearer_auth(session_token)
            .json(&StartRequestRequest {
                name: params.name,
                title: params.title,
                audience: params.audience,
            }),
        owner,
        repo,
        "start request",
    )
}

pub fn submit_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    execute_request(
        client
            .post(request_action_url(api_url, target, "submit"))
            .bearer_auth(session_token)
            .json(&SubmitRequestRequest {}),
        target,
        "submit request",
    )
}

pub fn merge_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    execute_request(
        client
            .post(request_action_url(api_url, target, "merge"))
            .bearer_auth(session_token),
        target,
        "merge request",
    )
}

pub fn rate_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
    score: u8,
    reason: String,
) -> anyhow::Result<RequestRatingResponse> {
    execute_request(
        client
            .post(request_action_url(api_url, target, "ratings"))
            .bearer_auth(session_token)
            .json(&CreateRequestRatingRequest { score, reason }),
        target,
        "rate request participant",
    )
}

pub fn edit_request_identity(
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
    title: Option<String>,
    description_markdown: Option<String>,
) -> anyhow::Result<RequestMutationResponse> {
    execute_request(
        client
            .patch(request_url(api_url, target))
            .bearer_auth(session_token)
            .json(&EditRequestIdentityRequest {
                title,
                description_markdown,
            }),
        target,
        "edit request identity",
    )
}

pub fn add_request_invitee(
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
    handle: String,
) -> anyhow::Result<RequestInviteeMutationResponse> {
    execute_request(
        client
            .put(request_action_url(api_url, target, "invitees"))
            .bearer_auth(session_token)
            .json(&AddRequestInviteeRequest { handle }),
        target,
        "invite request collaborator",
    )
}

pub fn remove_request_invitee(
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
    handle: String,
) -> anyhow::Result<RequestInviteeMutationResponse> {
    execute_request(
        client
            .delete(request_action_url(api_url, target, "invitees"))
            .bearer_auth(session_token)
            .json(&RemoveRequestInviteeRequest { handle }),
        target,
        "remove request invitee",
    )
}

pub fn leave_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
) -> anyhow::Result<LeaveRequestResponse> {
    execute_request(
        client
            .delete(format!(
                "{api_url}{}",
                scope_api_contract::routes::repo_request_invitees_me(
                    target.owner,
                    target.repo,
                    target.request_id,
                )
            ))
            .bearer_auth(session_token),
        target,
        "leave request",
    )
}

pub fn get_request_activity(
    client: &Client,
    api_url: &str,
    session_token: &str,
    params: RequestActivityParams<'_>,
) -> anyhow::Result<RequestActivityPageResponse> {
    let mut request = client
        .get(request_action_url(api_url, params.target, "activity"))
        .bearer_auth(session_token);
    if let Some(after) = params.after {
        request = request.query(&[("after", after)]);
    }
    if params.latest {
        request = request.query(&[("latest", true)]);
    }
    if let Some(limit) = params.limit {
        request = request.query(&[("limit", limit)]);
    }
    execute_request(request, params.target, "load request activity")
}

pub fn create_request_discussion(
    client: &Client,
    api_url: &str,
    session_token: &str,
    params: CreateRequestDiscussionParams<'_>,
) -> anyhow::Result<RequestDiscussionMutationResponse> {
    let target = RequestTarget {
        owner: params.owner,
        repo: params.repo,
        request_id: params.request_id,
    };
    execute_request(
        client
            .post(request_action_url(api_url, target, "timeline"))
            .bearer_auth(session_token)
            .json(&CreateRequestDiscussionRequest {
                body_markdown: params.body_markdown,
                client_discussion_id: params.client_discussion_id,
                anchor: None,
            }),
        target,
        "create request discussion",
    )
}

fn request_url(api_url: &str, target: RequestTarget<'_>) -> String {
    format!(
        "{api_url}{}",
        scope_api_contract::routes::repo_request(target.owner, target.repo, target.request_id)
    )
}

fn request_action_url(api_url: &str, target: RequestTarget<'_>, action: &str) -> String {
    format!(
        "{api_url}{}",
        scope_api_contract::routes::repo_request_action(
            target.owner,
            target.repo,
            target.request_id,
            action,
        )
    )
}

fn execute_repo_request<R: DeserializeOwned>(
    request: RequestBuilder,
    owner: &str,
    repo: &str,
    action: &str,
) -> anyhow::Result<R> {
    let context = format!("{action} for {owner}/{repo}");
    let response = request.send().with_context(|| context.clone())?;
    decode_json_response(response, &context)
}

fn execute_request<R: DeserializeOwned>(
    request: RequestBuilder,
    target: RequestTarget<'_>,
    action: &str,
) -> anyhow::Result<R> {
    let context = format!(
        "{action} {} for {}/{}",
        target.request_id, target.owner, target.repo
    );
    let response = request.send().with_context(|| context.clone())?;
    decode_json_response(response, &context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn list_requests_sends_the_opaque_cursor_as_a_query_parameter() {
        let (api_url, server) = serve_once(StatusCode::OK, r#"{"requests":[],"next_cursor":null}"#);

        let response = list_requests(
            &Client::new(),
            &api_url,
            "token",
            "owner",
            "repo",
            Some("next page/+"),
        )
        .unwrap();

        assert!(response.requests.is_empty());
        assert!(response.next_cursor.is_none());
        let request = server.join().unwrap();
        assert!(
            request
                .lines()
                .next()
                .unwrap()
                .ends_with("?cursor=next+page%2F%2B HTTP/1.1"),
            "{request}"
        );
    }

    #[test]
    fn request_errors_surface_authoritative_safe_messages() {
        let (api_url, server) = serve_once(
            StatusCode::CONFLICT,
            r#"{"code":"conflict","message":"request cannot be submitted\u001b[31m","retryable":false}"#,
        );

        let error = submit_request(&Client::new(), &api_url, "token", target())
            .unwrap_err()
            .to_string();

        assert_eq!(error, "request cannot be submitted [31m");
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/repos/owner/repo/requests/req_one/submit "));
        assert!(request.contains("\r\n\r\n{}"), "{request}");
    }

    #[test]
    fn diagnostic_errors_surface_only_the_safe_message_and_reference() {
        let (api_url, server) = serve_once(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"code":"internal","message":"Scope hit an internal error.","error_reference":"err_0123456789abcdef0123456789abcdef\u001b[31m","retryable":false}"#,
        );

        let error = get_request(
            &Client::new(),
            &api_url,
            "token",
            "owner",
            "repo",
            "req_one",
        )
        .unwrap_err()
        .to_string();

        assert_eq!(
            error,
            "Scope hit an internal error.\nReference: err_0123456789abcdef0123456789abcdef [31m"
        );
        server.join().unwrap();
    }

    #[test]
    fn request_errors_surface_the_cli_upgrade_instruction() {
        let (api_url, server) = serve_once(
            StatusCode::UPGRADE_REQUIRED,
            r#"{"code":"cli_upgrade_required","message":"installed Scope CLI protocol 0; this API supports protocol 1","instruction":"Upgrade with `curl -fsSL https://scope-cli-production.up.railway.app/install.sh | sh`, then retry.","fields":{"installed_protocol":0,"supported_protocol":1},"retryable":false}"#,
        );

        let error = submit_request(&Client::new(), &api_url, "token", target())
            .unwrap_err()
            .to_string();

        assert!(error.contains("installed Scope CLI protocol 0"), "{error}");
        assert!(error.contains(CLI_INSTALL_COMMAND), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn submit_posts_an_empty_payload() {
        let (api_url, server) = serve_once(
            StatusCode::CONFLICT,
            r#"{"code":"conflict","message":"fixture stop","retryable":false}"#,
        );

        submit_request(&Client::new(), &api_url, "token", target()).unwrap_err();

        let request = server.join().unwrap();
        assert!(request.contains("\r\n\r\n{}"), "{request}");
    }

    #[test]
    fn request_not_found_uses_the_authoritative_contract_message() {
        let (api_url, server) = serve_once(
            StatusCode::NOT_FOUND,
            r#"{"code":"not_found","message":"request req_one not found in owner/repo","retryable":false}"#,
        );

        let error = get_request(
            &Client::new(),
            &api_url,
            "token",
            "owner",
            "repo",
            "req_one",
        )
        .unwrap_err()
        .to_string();

        assert_eq!(error, "request req_one not found in owner/repo");
        server.join().unwrap();
    }

    #[test]
    fn malformed_error_bodies_use_a_scoped_status_fallback() {
        let (api_url, server) = serve_once(StatusCode::SERVICE_UNAVAILABLE, "upstream exploded");

        let error = merge_request(&Client::new(), &api_url, "token", target())
            .unwrap_err()
            .to_string();

        assert_eq!(
            error,
            "Scope is temporarily unavailable while trying to merge request req_one for owner/repo"
        );
        server.join().unwrap();
    }

    #[test]
    fn invite_and_activity_wrappers_use_contract_methods_queries_and_payloads() {
        let (api_url, invite_server) = serve_once(
            StatusCode::CONFLICT,
            r#"{"code":"conflict","message":"fixture stop","retryable":false}"#,
        );
        add_request_invitee(
            &Client::new(),
            &api_url,
            "token",
            target(),
            "Exact-Handle".to_string(),
        )
        .unwrap_err();
        let invite_request = invite_server.join().unwrap();
        assert!(
            invite_request
                .starts_with("PUT /v1/repos/owner/repo/requests/req_one/invitees HTTP/1.1")
        );
        assert!(
            invite_request.contains(r#"{"handle":"Exact-Handle"}"#),
            "{invite_request}"
        );

        let (api_url, activity_server) =
            serve_once(StatusCode::OK, r#"{"events":[],"through_position":7}"#);
        let page = get_request_activity(
            &Client::new(),
            &api_url,
            "token",
            RequestActivityParams {
                target: target(),
                after: Some(4),
                latest: true,
                limit: Some(25),
            },
        )
        .unwrap();
        assert!(page.events.is_empty());
        assert_eq!(page.through_position, 7);
        let activity_request = activity_server.join().unwrap();
        let request_line = activity_request.lines().next().unwrap();
        assert!(
            request_line.starts_with("GET /v1/repos/owner/repo/requests/req_one/activity?"),
            "{request_line}"
        );
        for query in ["after=4", "latest=true", "limit=25"] {
            assert!(request_line.contains(query), "{request_line}");
        }
    }

    fn target() -> RequestTarget<'static> {
        RequestTarget {
            owner: "owner",
            repo: "repo",
            request_id: "req_one",
        }
    }

    fn serve_once(status: StatusCode, body: &'static str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 8192];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8(request[..read].to_vec()).unwrap();
            write!(
                stream,
                "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("Unknown"),
                body.len(),
            )
            .unwrap();
            request
        });
        (format!("http://{address}"), server)
    }
}
