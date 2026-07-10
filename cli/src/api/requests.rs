use super::*;
use serde::Serialize;

pub struct StartRequestParams<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub title: String,
}

pub struct SubmitRequestParams<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub request_id: &'a str,
    pub head_oid: String,
    pub stake_credits: Option<u32>,
}

pub struct ResolveRequestParams<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub request_id: &'a str,
    pub disposition: ResolutionDisposition,
    pub body: Option<String>,
}

pub struct MergeRequestParams<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub request_id: &'a str,
    pub expected_main_oid: String,
    pub expected_head_oid: String,
    pub body: Option<String>,
}

pub fn list_requests(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
) -> anyhow::Result<RequestListResponse> {
    let response = client
        .get(format!(
            "{api_url}{}",
            scope_api_contract::routes::repo_requests(owner, repo)
        ))
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

pub fn download_request_branch_bundle(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    request_id: &str,
) -> anyhow::Result<Vec<u8>> {
    let response = client
        .get(format!(
            "{api_url}{}",
            scope_api_contract::routes::repo_request_action(
                owner,
                repo,
                request_id,
                "branch.bundle"
            )
        ))
        .bearer_auth(session_token)
        .send()
        .with_context(|| format!("download request branch for {owner}/{repo}#{request_id}"))?;
    handle_request_status(
        response.status(),
        owner,
        repo,
        request_id,
        "download request branch",
    )?;
    Ok(response
        .error_for_status()
        .with_context(|| format!("download request branch for {owner}/{repo}#{request_id}"))?
        .bytes()
        .context("read request branch bundle")?
        .to_vec())
}

pub fn delete_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    request_id: &str,
) -> anyhow::Result<RequestDeleteResponse> {
    let response = client
        .delete(format!(
            "{api_url}{}",
            scope_api_contract::routes::repo_request(owner, repo, request_id)
        ))
        .bearer_auth(session_token)
        .send()
        .with_context(|| format!("delete request {request_id} for {owner}/{repo}"))?;
    handle_request_status(response.status(), owner, repo, request_id, "delete request")?;
    response
        .error_for_status()
        .with_context(|| format!("delete request {request_id} for {owner}/{repo}"))?
        .json()
        .context("parse delete request response")
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
            title: params.title,
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

pub fn submit_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    params: SubmitRequestParams<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    let response = client
        .post(format!(
            "{api_url}{}",
            scope_api_contract::routes::repo_request_action(
                params.owner,
                params.repo,
                params.request_id,
                "submit"
            )
        ))
        .bearer_auth(session_token)
        .json(&SubmitRequestRequest {
            head_oid: params.head_oid,
            stake_credits: params.stake_credits,
        })
        .send()
        .with_context(|| {
            format!(
                "submit request {} for {}/{}",
                params.request_id, params.owner, params.repo
            )
        })?;
    handle_repo_request_status(
        response.status(),
        params.owner,
        params.repo,
        "submit request",
    )?;
    response
        .error_for_status()
        .with_context(|| {
            format!(
                "submit request {} for {}/{}",
                params.request_id, params.owner, params.repo
            )
        })?
        .json()
        .context("parse submit request response")
}

pub fn comment_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    request_id: &str,
    body: String,
) -> anyhow::Result<RequestMutationResponse> {
    request_mutation(
        client,
        api_url,
        session_token,
        RequestMutationEndpoint {
            owner,
            repo,
            request_id,
            action_path: "comments",
            context: "comment request",
        },
        &CommentRequestRequest { body },
    )
}

pub fn mark_request_needs_response(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    request_id: &str,
    body: String,
) -> anyhow::Result<RequestMutationResponse> {
    request_mutation(
        client,
        api_url,
        session_token,
        RequestMutationEndpoint {
            owner,
            repo,
            request_id,
            action_path: "needs-response",
            context: "mark request needs response",
        },
        &NeedsResponseRequest { body },
    )
}

pub fn respond_to_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    owner: &str,
    repo: &str,
    request_id: &str,
    body: Option<String>,
) -> anyhow::Result<RequestMutationResponse> {
    request_mutation(
        client,
        api_url,
        session_token,
        RequestMutationEndpoint {
            owner,
            repo,
            request_id,
            action_path: "respond",
            context: "respond to request",
        },
        &RespondRequestRequest { body },
    )
}

pub fn resolve_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    params: ResolveRequestParams<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    request_mutation(
        client,
        api_url,
        session_token,
        RequestMutationEndpoint {
            owner: params.owner,
            repo: params.repo,
            request_id: params.request_id,
            action_path: "resolve",
            context: "resolve request",
        },
        &ResolveRequestRequest {
            disposition: params.disposition,
            body: params.body,
        },
    )
}

pub fn merge_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    params: MergeRequestParams<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    request_mutation(
        client,
        api_url,
        session_token,
        RequestMutationEndpoint {
            owner: params.owner,
            repo: params.repo,
            request_id: params.request_id,
            action_path: "merge",
            context: "merge request",
        },
        &MergeRequestRequest {
            expected_main_oid: params.expected_main_oid,
            expected_head_oid: params.expected_head_oid,
            body: params.body,
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

fn request_mutation<T: Serialize>(
    client: &Client,
    api_url: &str,
    session_token: &str,
    endpoint: RequestMutationEndpoint<'_>,
    body: &T,
) -> anyhow::Result<RequestMutationResponse> {
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
