use crate::{
    api::{RepoSummaryResponse, RepositoryActor, RequestSummaryResponse, get_repo, list_requests},
    git_repo::{
        GitRepo, branch_config_value, changed_paths_since_scope_base_at_commit, current_branch,
        fetch_scope_remote_with_bearer, push_head_to_ref_with_bearer, run_git_in_repo,
        set_branch_config_value,
    },
    push::DEFAULT_SCOPE_BRANCH,
    request::remote::{
        REQUEST_REMOTE_KEY, RequestRemoteTarget, load_request_remote, request_remote_name,
    },
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_core::domain::requests::RequestAudience;

const REQUEST_ID_KEY: &str = "scopeRequestId";
const REQUEST_OWNER_KEY: &str = "scopeRequestOwner";
const REQUEST_REPO_KEY: &str = "scopeRequestRepo";
const REQUEST_AUDIENCE_KEY: &str = "scopeRequestAudience";

pub(super) struct RequestContext {
    pub(super) target: RequestRemoteTarget,
    pub(super) repo: RepoSummaryResponse,
}

pub(super) fn load_context(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<&str>,
) -> anyhow::Result<RequestContext> {
    let remote = request_remote_name(git_repo, api_url, remote)?;
    let target = load_request_remote(git_repo, api_url, &remote)?;
    let repo = get_repo(client, api_url, session_token, &target.owner, &target.repo)?;
    Ok(RequestContext { target, repo })
}

pub(super) fn load_context_and_request_id(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
) -> anyhow::Result<(RequestContext, String)> {
    let context = load_context(git_repo, client, api_url, session_token, remote.as_deref())?;
    crate::request::render::print_repo_access(&context.repo);
    let request_id = request_id_for_context(
        git_repo,
        client,
        api_url,
        session_token,
        &context,
        request_id,
    )?;
    Ok((context, request_id))
}

pub(super) fn fetch_main_projection(
    git_repo: &GitRepo,
    context: &RequestContext,
    audience: RequestAudience,
    session_token: &str,
) -> anyhow::Result<()> {
    let fetch_url = match audience {
        RequestAudience::Public => &context.target.public_url,
        RequestAudience::Private => &context.target.permissioned_url,
    };
    fetch_scope_remote_with_bearer(
        git_repo,
        fetch_url,
        &context.target.remote,
        DEFAULT_SCOPE_BRANCH,
        session_token,
    )
}

pub(super) fn normalized_submit_stake(
    repo: &RepoSummaryResponse,
    stake_credits: Option<u32>,
) -> anyhow::Result<Option<u32>> {
    if !repo.request_permissions.can_submit_request {
        bail!(
            "this user cannot submit requests to {}/{}",
            repo.owner_handle,
            repo.name
        );
    }
    match repo.access.actor {
        RepositoryActor::Public => match stake_credits {
            Some(stake) if stake > 0 => Ok(Some(stake)),
            _ => bail!("public request submission requires --stake-credits greater than 0"),
        },
        RepositoryActor::Member | RepositoryActor::Owner => {
            if stake_credits.unwrap_or(0) != 0 {
                bail!(
                    "owner/member request submission does not use credit stake; omit --stake-credits"
                );
            }
            Ok(None)
        }
    }
}

pub(super) fn push_request_head(
    target: &RequestRemoteTarget,
    session_token: &str,
    request_head_oid: &str,
    request_id: &str,
    request_name: &str,
) -> anyhow::Result<()> {
    let request_ref = format!("refs/heads/{request_name}");
    push_head_to_ref_with_bearer(
        &target.permissioned_url,
        request_head_oid,
        &request_ref,
        session_token,
    )
    .with_context(|| format!("push request branch for {request_id}"))
}

pub(super) fn request_id_for_context(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    context: &RequestContext,
    request_id: Option<String>,
) -> anyhow::Result<String> {
    maybe_request_id_for_context(
        git_repo,
        client,
        api_url,
        session_token,
        context,
        request_id,
    )?
    .context("current branch is not a visible Scope request; switch to origin/<request-name> or pass a request id")
}

pub(super) fn maybe_request_id_for_context(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    context: &RequestContext,
    request_id: Option<String>,
) -> anyhow::Result<Option<String>> {
    let explicit = normalized_optional_arg(request_id);
    if let Some(request_id) = explicit
        .as_deref()
        .filter(|value| value.starts_with("req_"))
    {
        return Ok(Some(request_id.to_string()));
    }
    let branch = current_branch(git_repo)?;
    if explicit.is_none()
        && let Some(request_id) = branch_config_value(git_repo, &branch, REQUEST_ID_KEY)?
    {
        return Ok(Some(request_id));
    }
    let request_name = explicit.as_deref().unwrap_or(&branch);
    let requests = list_requests(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
    )?;
    Ok(requests
        .requests
        .into_iter()
        .find(|request| request.name == request_name)
        .map(|request| request.id))
}

pub(super) fn maybe_request_branch_audience(
    git_repo: &GitRepo,
) -> anyhow::Result<Option<RequestAudience>> {
    let branch = current_branch(git_repo)?;
    branch_config_value(git_repo, &branch, REQUEST_AUDIENCE_KEY)?
        .map(|value| parse_audience_config(&value))
        .transpose()
}

pub(super) fn store_branch_context(
    git_repo: &GitRepo,
    branch: &str,
    context: &RequestContext,
) -> anyhow::Result<()> {
    set_branch_config_value(git_repo, branch, REQUEST_OWNER_KEY, &context.target.owner)?;
    set_branch_config_value(git_repo, branch, REQUEST_REPO_KEY, &context.target.repo)?;
    set_branch_config_value(git_repo, branch, REQUEST_REMOTE_KEY, &context.target.remote)?;
    Ok(())
}

pub(super) fn store_request_metadata(
    git_repo: &GitRepo,
    branch: &str,
    context: &RequestContext,
    request: &RequestSummaryResponse,
) -> anyhow::Result<()> {
    store_branch_context(git_repo, branch, context)?;
    store_request_metadata_fields(git_repo, branch, &request.id, request.audience)
}

pub(super) fn store_request_metadata_fields(
    git_repo: &GitRepo,
    branch: &str,
    request_id: &str,
    audience: RequestAudience,
) -> anyhow::Result<()> {
    set_branch_config_value(git_repo, branch, REQUEST_ID_KEY, request_id)?;
    set_branch_config_value(
        git_repo,
        branch,
        REQUEST_AUDIENCE_KEY,
        audience_config_value(audience),
    )?;
    Ok(())
}

pub(super) fn track_request_branch_ref(
    git_repo: &GitRepo,
    branch: &str,
    target: &RequestRemoteTarget,
    request_name: &str,
    request_head_oid: &str,
) -> anyhow::Result<()> {
    let remote_ref = request_remote_ref(&target.remote, request_name);
    run_git_in_repo(git_repo, &["update-ref", &remote_ref, request_head_oid])?;
    set_branch_config_value(git_repo, branch, "remote", &target.remote)?;
    set_branch_config_value(
        git_repo,
        branch,
        "merge",
        &format!("refs/heads/{request_name}"),
    )
}

pub(super) fn print_change_summary(
    git_repo: &GitRepo,
    target: &RequestRemoteTarget,
    request_head_oid: &str,
) -> anyhow::Result<()> {
    let remote_main = remote_main_ref(&target.remote);
    let changes =
        changed_paths_since_scope_base_at_commit(git_repo, Some(&remote_main), request_head_oid)?;
    if changes.is_empty() {
        println!("Committed diff: no file changes from {remote_main}");
        return Ok(());
    }

    println!(
        "Committed diff: {} changed file(s) from {remote_main}",
        changes.len()
    );
    for change in changes.iter().take(12) {
        println!("  {} {}", change.status, change.path);
    }
    if changes.len() > 12 {
        println!("  ... {} more", changes.len() - 12);
    }
    Ok(())
}

pub(super) fn projection_label_for_audience(audience: RequestAudience) -> &'static str {
    match audience {
        RequestAudience::Public => "public main",
        RequestAudience::Private => "private main",
    }
}

pub(super) fn remote_main_ref(remote: &str) -> String {
    format!("refs/remotes/{remote}/{DEFAULT_SCOPE_BRANCH}")
}

pub(super) fn request_remote_ref(remote: &str, request_name: &str) -> String {
    format!("refs/remotes/{remote}/{request_name}")
}

fn audience_config_value(audience: RequestAudience) -> &'static str {
    match audience {
        RequestAudience::Public => "public",
        RequestAudience::Private => "private",
    }
}

pub(super) fn parse_audience_config(value: &str) -> anyhow::Result<RequestAudience> {
    match value {
        "public" => Ok(RequestAudience::Public),
        "private" => Ok(RequestAudience::Private),
        _ => bail!("invalid Scope request base audience '{value}'"),
    }
}

fn normalized_optional_arg(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
