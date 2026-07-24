use super::*;
use serde::{Serialize, de::DeserializeOwned};

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

pub fn list_requests(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    cursor: Option<&str>,
) -> anyhow::Result<RequestListResponse> {
    let mut request = client.get(format!(
        "{api_url}{}",
        scope_api_contract::routes::repo_requests(owner, repo)
    ));
    if let Some(cursor) = cursor {
        request = request.query(&[("cursor", cursor)]);
    }
    let response = request
        .bearer_auth(session_token)
        .send()
        .with_context(|| format!("list requests for {owner}/{repo}"))?;
    handle_repo_request_status(response.status(), owner, repo, "list requests")?;
    response
        .error_for_status()
        .with_context(|| format!("list requests for {owner}/{repo}"))?
        .json()
        .context("parse request list response")
}

pub fn get_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    request_id: &str,
) -> anyhow::Result<RequestDetailResponse> {
    let response = client
        .get(format!(
            "{api_url}{}",
            scope_api_contract::routes::repo_request(owner, repo, request_id)
        ))
        .bearer_auth(session_token)
        .send()
        .with_context(|| format!("load request {request_id} for {owner}/{repo}"))?;
    handle_request_status(response.status(), owner, repo, request_id, "load request")?;
    response
        .error_for_status()
        .with_context(|| format!("load request {request_id} for {owner}/{repo}"))?
        .json()
        .context("parse request detail response")
}

pub fn close_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    request_id: &str,
) -> anyhow::Result<RequestCloseResponse> {
    let response = client
        .delete(format!(
            "{api_url}{}",
            scope_api_contract::routes::repo_request(owner, repo, request_id)
        ))
        .bearer_auth(session_token)
        .send()
        .with_context(|| format!("close request {request_id} for {owner}/{repo}"))?;
    handle_request_status(response.status(), owner, repo, request_id, "close request")?;
    response
        .error_for_status()
        .with_context(|| format!("close request {request_id} for {owner}/{repo}"))?
        .json()
        .context("parse close request response")
}

pub fn start_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    params: StartRequestParams<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    let response = client
        .post(format!(
            "{api_url}{}",
            scope_api_contract::routes::repo_requests(params.owner, params.repo)
        ))
        .bearer_auth(session_token)
        .json(&StartRequestRequest {
            name: params.name,
            title: params.title,
            audience: params.audience,
        })
        .send()
        .with_context(|| format!("start request for {}/{}", params.owner, params.repo))?;
    handle_repo_request_status(
        response.status(),
        params.owner,
        params.repo,
        "start request",
    )?;
    response
        .error_for_status()
        .with_context(|| format!("start request for {}/{}", params.owner, params.repo))?
        .json()
        .context("parse start request response")
}

pub fn create_request_discussion(
    client: &Client,
    api_url: &str,
    session_token: &str,
    params: CreateRequestDiscussionParams<'_>,
) -> anyhow::Result<RequestDiscussionMutationResponse> {
    request_mutation(
        client,
        api_url,
        session_token,
        RequestMutationEndpoint {
            owner: params.owner,
            repo: params.repo,
            request_id: params.request_id,
            action_path: "discussions",
            context: "create request discussion",
        },
        &CreateRequestDiscussionRequest {
            body_markdown: params.body_markdown,
            client_discussion_id: params.client_discussion_id,
        },
    )
}

struct RequestMutationEndpoint<'a> {
    owner: &'a str,
    repo: &'a str,
    request_id: &'a str,
    action_path: &'static str,
    context: &'static str,
}

fn request_mutation<T: Serialize, R: DeserializeOwned>(
    client: &Client,
    api_url: &str,
    session_token: &str,
    endpoint: RequestMutationEndpoint<'_>,
    body: &T,
) -> anyhow::Result<R> {
    let response = client
        .post(format!(
            "{api_url}{}",
            scope_api_contract::routes::repo_request_action(
                endpoint.owner,
                endpoint.repo,
                endpoint.request_id,
                endpoint.action_path
            )
        ))
        .bearer_auth(session_token)
        .json(body)
        .send()
        .with_context(|| {
            format!(
                "{} {} for {}/{}",
                endpoint.context, endpoint.request_id, endpoint.owner, endpoint.repo
            )
        })?;
    handle_request_status(
        response.status(),
        endpoint.owner,
        endpoint.repo,
        endpoint.request_id,
        endpoint.context,
    )?;
    response
        .error_for_status()
        .with_context(|| {
            format!(
                "{} {} for {}/{}",
                endpoint.context, endpoint.request_id, endpoint.owner, endpoint.repo
            )
        })?
        .json()
        .with_context(|| format!("parse {} response", endpoint.context))
}

fn handle_repo_request_status(
    status: StatusCode,
    owner: &str,
    repo: &str,
    action: &str,
) -> anyhow::Result<()> {
    match status {
        StatusCode::UNAUTHORIZED => anyhow::bail!("not signed in; run scope login"),
        StatusCode::FORBIDDEN => anyhow::bail!("{action} is not allowed for {owner}/{repo}"),
        StatusCode::NOT_FOUND => anyhow::bail!("repo {owner}/{repo} not found"),
        StatusCode::CONFLICT => anyhow::bail!("{action} conflicted for {owner}/{repo}"),
        _ => Ok(()),
    }
}

fn handle_request_status(
    status: StatusCode,
    owner: &str,
    repo: &str,
    request_id: &str,
    action: &str,
) -> anyhow::Result<()> {
    match status {
        StatusCode::UNAUTHORIZED => anyhow::bail!("not signed in; run scope login"),
        StatusCode::FORBIDDEN => anyhow::bail!("{action} is not allowed for request {request_id}"),
        StatusCode::NOT_FOUND => {
            anyhow::bail!("request {request_id} not found in {owner}/{repo}")
        }
        StatusCode::CONFLICT => anyhow::bail!("{action} conflicted for request {request_id}"),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::list_requests;
    use reqwest::blocking::Client;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn list_requests_sends_the_opaque_cursor_as_a_query_parameter() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(
                request
                    .lines()
                    .next()
                    .unwrap()
                    .ends_with("?cursor=next+page%2F%2B HTTP/1.1"),
                "{request}"
            );

            let body = r#"{"requests":[],"next_cursor":null}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });

        let response = list_requests(
            &Client::new(),
            &format!("http://{address}"),
            "token",
            "owner",
            "repo",
            Some("next page/+"),
        )
        .unwrap();

        assert!(response.requests.is_empty());
        assert!(response.next_cursor.is_none());
        server.join().unwrap();
    }
}
