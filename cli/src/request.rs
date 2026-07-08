use crate::{
    api::{
        FinalizeRequestSubmissionParams, MergeRequestParams, RepoSummaryResponse, RepositoryActor,
        RequestSummaryResponse, ResolveRequestParams, comment_request, finalize_request_submission,
        get_repo, get_request, list_requests, mark_request_needs_response, merge_request,
        reserve_request, resolve_request, respond_to_request,
    },
    git_repo::{
        GitRepo, branch_config_value, changed_paths_since_scope_base_at_commit, current_branch,
        ensure_clean_working_tree, ensure_git_repo_ready, fetch_scope_remote_with_bearer, head_oid,
        push_head_to_ref_with_bearer, run_git_in_repo, scope_remote_head_oid,
        set_branch_config_value, warn_if_dirty_working_tree,
    },
    push::DEFAULT_SCOPE_BRANCH,
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_core::domain::requests::{RequestBaseAudience, RequestDisposition};
use std::time::{SystemTime, UNIX_EPOCH};

mod args;
mod remote;
mod render;
#[cfg(test)]
mod tests;
mod text;
pub use args::RequestArgs;
use args::RequestCommand;
use remote::{REQUEST_REMOTE_KEY, RequestRemoteTarget, load_request_remote, request_remote_name};
use render::{
    confirm_merge, ensure_mergeable, print_mutation_receipt, print_repo_access,
    print_request_detail, print_submit_stake, request_line,
};
use text::short_oid;

const REQUEST_ID_KEY: &str = "scopeRequestId";
const REQUEST_REF_KEY: &str = "scopeRequestRef";
const REQUEST_OWNER_KEY: &str = "scopeRequestOwner";
const REQUEST_REPO_KEY: &str = "scopeRequestRepo";
const REQUEST_BASE_AUDIENCE_KEY: &str = "scopeRequestBaseAudience";

pub fn run_request_command(
    args: RequestArgs,
    client: &Client,
    api_url: &str,
    session_token: &str,
) -> anyhow::Result<()> {
    match args.command {
        RequestCommand::Start(args) => {
            start_request(client, api_url, session_token, args.remote, args.branch)
        }
        RequestCommand::Submit(args) => submit_request_branch(
            client,
            api_url,
            session_token,
            args.remote,
            args.title,
            args.stake_credits,
        ),
        RequestCommand::Update(args) => {
            update_request_branch(client, api_url, session_token, args.remote, args.id)
        }
        RequestCommand::Sync(args) => {
            sync_request_branch(client, api_url, session_token, args.remote)
        }
        RequestCommand::Status(args) => {
            show_request_status(client, api_url, session_token, args.remote, args.id)
        }
        RequestCommand::Comment(args) => comment_on_request(
            client,
            api_url,
            session_token,
            args.remote,
            args.id,
            args.body,
        ),
        RequestCommand::NeedsResponse(args) => mark_needs_response(
            client,
            api_url,
            session_token,
            args.remote,
            args.id,
            args.body,
        ),
        RequestCommand::Respond(args) => respond_to_request_thread(
            client,
            api_url,
            session_token,
            args.remote,
            args.id,
            args.body,
        ),
        RequestCommand::Resolve(args) => resolve_request_thread(
            client,
            api_url,
            session_token,
            args.remote,
            args.id,
            args.disposition.into(),
            args.body,
        ),
        RequestCommand::Merge(args) => merge_request_thread(
            client,
            api_url,
            session_token,
            args.remote,
            args.id,
            args.body,
            args.yes,
        ),
    }
}

pub fn preflight_request_command(args: &RequestArgs) -> anyhow::Result<()> {
    match &args.command {
        RequestCommand::Start(_) => {
            let git_repo = ensure_git_repo_ready("scope request start")?;
            ensure_clean_working_tree(&git_repo, "scope request start")
        }
        RequestCommand::Sync(_) => {
            let git_repo = ensure_git_repo_ready("scope request sync")?;
            ensure_clean_working_tree(&git_repo, "scope request sync")?;
            ensure_request_branch_context(&git_repo, "scope request sync")
        }
        RequestCommand::Submit(_) => {
            let git_repo = ensure_git_repo_ready("scope request submit")?;
            ensure_branch_not_attached_to_request(&git_repo)
        }
        RequestCommand::Update(_) => {
            ensure_git_repo_ready("scope request update")?;
            Ok(())
        }
        RequestCommand::Status(_) => {
            ensure_git_repo_ready("scope request status")?;
            Ok(())
        }
        RequestCommand::Comment(_) => {
            ensure_git_repo_ready("scope request comment")?;
            Ok(())
        }
        RequestCommand::NeedsResponse(_) => {
            ensure_git_repo_ready("scope request needs-response")?;
            Ok(())
        }
        RequestCommand::Respond(_) => {
            ensure_git_repo_ready("scope request respond")?;
            Ok(())
        }
        RequestCommand::Resolve(_) => {
            ensure_git_repo_ready("scope request resolve")?;
            Ok(())
        }
        RequestCommand::Merge(_) => {
            ensure_git_repo_ready("scope request merge")?;
            Ok(())
        }
    }
}

fn start_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    branch: Option<String>,
) -> anyhow::Result<()> {
    let git_repo = ensure_git_repo_ready("scope request start")?;
    ensure_clean_working_tree(&git_repo, "scope request start")?;
    let context = load_context(&git_repo, client, api_url, session_token, remote.as_deref())?;
    print_repo_access(&context.repo);
    fetch_main_projection(
        &git_repo,
        &context,
        base_audience_for_repo(&context.repo),
        session_token,
    )?;
    let branch = branch
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(default_request_branch_name);
    let remote_main = remote_main_ref(&context.target.remote);
    let base_oid = scope_remote_head_oid(&git_repo, &context.target.remote, DEFAULT_SCOPE_BRANCH)?
        .context("Scope main projection did not produce a local remote ref")?;

    run_git_in_repo(&git_repo, &["switch", "-c", &branch, &remote_main])?;
    store_branch_context(&git_repo, &branch, &context)?;

    println!(
        "Created request branch {branch} from {} ({})",
        projection_label_for_repo(&context.repo),
        short_oid(&base_oid)
    );
    println!("Next: commit changes, then run scope request submit --title \"...\"");
    println!("Useful while working: scope request sync, scope request status");
    Ok(())
}

fn submit_request_branch(
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    title: String,
    stake_credits: Option<u32>,
) -> anyhow::Result<()> {
    let git_repo = ensure_git_repo_ready("scope request submit")?;
    ensure_branch_not_attached_to_request(&git_repo)?;
    warn_if_dirty_working_tree(&git_repo)?;
    let context = load_context(&git_repo, client, api_url, session_token, remote.as_deref())?;
    print_repo_access(&context.repo);
    let stake_credits = normalized_submit_stake(&context.repo, stake_credits)?;
    print_submit_stake(stake_credits);
    let base_audience = maybe_request_branch_base_audience(&git_repo)?
        .unwrap_or_else(|| base_audience_for_repo(&context.repo));
    fetch_main_projection(&git_repo, &context, base_audience, session_token)?;
    let request_head_oid = head_oid(&git_repo)?;
    print_change_summary(&git_repo, &context.target, &request_head_oid)?;

    let reservation = reserve_request(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
    )?;
    push_request_head(
        &context.target,
        session_token,
        &request_head_oid,
        &reservation.id,
        &reservation.request_ref,
    )
    .with_context(|| {
        format!(
            "reserved request {} was not submitted because its branch was not pushed; retry scope request submit",
            reservation.id
        )
    })?;
    let response = finalize_request_submission(
        client,
        api_url,
        session_token,
        FinalizeRequestSubmissionParams {
            owner: &context.target.owner,
            repo: &context.target.repo,
            request_id: &reservation.id,
            title,
            head_oid: request_head_oid.clone(),
            stake_credits,
        },
    )?;
    let branch = current_branch(&git_repo)?;
    store_request_metadata(&git_repo, &branch, &context, &response.request)?;

    println!(
        "Created request {} at {}",
        response.request.id, response.request.request_ref
    );
    let detail = get_request(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &response.request.id,
    )?;
    print_request_detail(&detail);
    Ok(())
}

fn update_request_branch(
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
) -> anyhow::Result<()> {
    let git_repo = ensure_git_repo_ready("scope request update")?;
    warn_if_dirty_working_tree(&git_repo)?;
    let context = load_context(&git_repo, client, api_url, session_token, remote.as_deref())?;
    print_repo_access(&context.repo);
    let request_id = current_or_explicit_request_id(&git_repo, request_id)?;
    let detail = get_request(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )?;
    if !detail.request.permissions.can_update_branch {
        bail!(
            "request {} cannot be updated by this user",
            detail.request.id
        );
    }
    let request_head_oid = head_oid(&git_repo)?;
    push_request_head(
        &context.target,
        session_token,
        &request_head_oid,
        &detail.request.id,
        &detail.request.request_ref,
    )?;
    let branch = current_branch(&git_repo)?;
    store_request_metadata(&git_repo, &branch, &context, &detail.request)?;
    let detail = get_request(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )?;
    print_request_detail(&detail);
    Ok(())
}

fn sync_request_branch(
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
) -> anyhow::Result<()> {
    let git_repo = ensure_git_repo_ready("scope request sync")?;
    ensure_clean_working_tree(&git_repo, "scope request sync")?;
    ensure_request_branch_context(&git_repo, "scope request sync")?;
    let context = load_context(&git_repo, client, api_url, session_token, remote.as_deref())?;
    print_repo_access(&context.repo);
    let base_audience = request_branch_base_audience(&git_repo)?;
    fetch_main_projection(&git_repo, &context, base_audience, session_token)?;
    let remote_main = remote_main_ref(&context.target.remote);
    run_git_in_repo(&git_repo, &["rebase", &remote_main])?;
    let base_oid = scope_remote_head_oid(&git_repo, &context.target.remote, DEFAULT_SCOPE_BRANCH)?
        .context("Scope main projection did not produce a local remote ref")?;
    println!(
        "Rebased {} onto latest {} ({})",
        current_branch(&git_repo)?,
        projection_label_for_repo(&context.repo),
        short_oid(&base_oid)
    );
    Ok(())
}

fn show_request_status(
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
) -> anyhow::Result<()> {
    let git_repo = ensure_git_repo_ready("scope request status")?;
    let context = load_context(&git_repo, client, api_url, session_token, remote.as_deref())?;
    print_repo_access(&context.repo);
    if let Some(request_id) = maybe_current_or_explicit_request_id(&git_repo, request_id)? {
        let detail = get_request(
            client,
            api_url,
            session_token,
            &context.target.owner,
            &context.target.repo,
            &request_id,
        )?;
        print_request_detail(&detail);
        return Ok(());
    }

    let response = list_requests(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
    )?;
    if response.requests.is_empty() {
        println!("No visible requests.");
        return Ok(());
    }
    println!("Visible requests:");
    for request in response.requests {
        println!("  {}", request_line(&request));
    }
    Ok(())
}

fn comment_on_request(
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
    body: String,
) -> anyhow::Result<()> {
    let (context, request_id) = load_context_and_request_id(
        client,
        api_url,
        session_token,
        remote,
        request_id,
        "scope request comment",
    )?;
    let response = comment_request(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &request_id,
        body,
    )?;
    print_mutation_receipt("Comment added", &response);
    Ok(())
}

fn mark_needs_response(
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
    body: String,
) -> anyhow::Result<()> {
    let (context, request_id) = load_context_and_request_id(
        client,
        api_url,
        session_token,
        remote,
        request_id,
        "scope request needs-response",
    )?;
    let response = mark_request_needs_response(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &request_id,
        body,
    )?;
    print_mutation_receipt("Request marked needs-response", &response);
    Ok(())
}

fn respond_to_request_thread(
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
    body: Option<String>,
) -> anyhow::Result<()> {
    let (context, request_id) = load_context_and_request_id(
        client,
        api_url,
        session_token,
        remote,
        request_id,
        "scope request respond",
    )?;
    let response = respond_to_request(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &request_id,
        body,
    )?;
    print_mutation_receipt("Response recorded", &response);
    Ok(())
}

fn resolve_request_thread(
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
    disposition: RequestDisposition,
    body: Option<String>,
) -> anyhow::Result<()> {
    let (context, request_id) = load_context_and_request_id(
        client,
        api_url,
        session_token,
        remote,
        request_id,
        "scope request resolve",
    )?;
    let response = resolve_request(
        client,
        api_url,
        session_token,
        ResolveRequestParams {
            owner: &context.target.owner,
            repo: &context.target.repo,
            request_id: &request_id,
            disposition,
            body,
        },
    )?;
    print_mutation_receipt("Request resolved", &response);
    Ok(())
}

fn merge_request_thread(
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
    body: Option<String>,
    yes: bool,
) -> anyhow::Result<()> {
    let (context, request_id) = load_context_and_request_id(
        client,
        api_url,
        session_token,
        remote,
        request_id,
        "scope request merge",
    )?;
    let detail = get_request(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )?;
    ensure_mergeable(&detail.request)?;
    if !yes {
        confirm_merge(&detail.request)?;
    }
    let expected_main_oid = detail
        .request
        .mergeability
        .current_main_oid
        .clone()
        .context("request has no current main oid to merge into")?;
    let response = merge_request(
        client,
        api_url,
        session_token,
        MergeRequestParams {
            owner: &context.target.owner,
            repo: &context.target.repo,
            request_id: &request_id,
            expected_main_oid,
            expected_head_oid: detail.request.mergeability.request_head_oid.clone(),
            body,
        },
    )?;
    print_mutation_receipt("Request merged", &response);
    Ok(())
}

struct RequestContext {
    target: RequestRemoteTarget,
    repo: RepoSummaryResponse,
}

fn load_context(
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

fn load_context_and_request_id(
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
    command_name: &str,
) -> anyhow::Result<(RequestContext, String)> {
    let git_repo = ensure_git_repo_ready(command_name)?;
    let context = load_context(&git_repo, client, api_url, session_token, remote.as_deref())?;
    print_repo_access(&context.repo);
    let request_id = current_or_explicit_request_id(&git_repo, request_id)?;
    Ok((context, request_id))
}

fn fetch_main_projection(
    git_repo: &GitRepo,
    context: &RequestContext,
    base_audience: RequestBaseAudience,
    session_token: &str,
) -> anyhow::Result<()> {
    let fetch_url = match base_audience {
        RequestBaseAudience::Public => &context.target.public_url,
        RequestBaseAudience::Private => &context.target.permissioned_url,
    };
    fetch_scope_remote_with_bearer(
        git_repo,
        fetch_url,
        &context.target.remote,
        DEFAULT_SCOPE_BRANCH,
        session_token,
    )
}

fn normalized_submit_stake(
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

fn push_request_head(
    target: &RequestRemoteTarget,
    session_token: &str,
    request_head_oid: &str,
    request_id: &str,
    request_ref: &str,
) -> anyhow::Result<()> {
    push_head_to_ref_with_bearer(
        &target.permissioned_url,
        request_head_oid,
        request_ref,
        session_token,
    )
    .with_context(|| format!("push request branch for {request_id}"))
}

fn current_or_explicit_request_id(
    git_repo: &GitRepo,
    request_id: Option<String>,
) -> anyhow::Result<String> {
    maybe_current_or_explicit_request_id(git_repo, request_id)?
        .context("no request id supplied and current branch is not attached to a Scope request")
}

fn maybe_current_or_explicit_request_id(
    git_repo: &GitRepo,
    request_id: Option<String>,
) -> anyhow::Result<Option<String>> {
    if let Some(request_id) = normalized_optional_arg(request_id) {
        return Ok(Some(request_id));
    }
    let branch = current_branch(git_repo)?;
    branch_config_value(git_repo, &branch, REQUEST_ID_KEY)
}

fn maybe_request_branch_base_audience(
    git_repo: &GitRepo,
) -> anyhow::Result<Option<RequestBaseAudience>> {
    let branch = current_branch(git_repo)?;
    branch_config_value(git_repo, &branch, REQUEST_BASE_AUDIENCE_KEY)?
        .map(|value| parse_base_audience_config(&value))
        .transpose()
}

fn request_branch_base_audience(git_repo: &GitRepo) -> anyhow::Result<RequestBaseAudience> {
    maybe_request_branch_base_audience(git_repo)?.context(
        "current branch is missing Scope request base audience; run scope request start first",
    )
}

fn ensure_branch_not_attached_to_request(git_repo: &GitRepo) -> anyhow::Result<()> {
    let branch = current_branch(git_repo)?;
    if let Some(request_id) = branch_config_value(git_repo, &branch, REQUEST_ID_KEY)? {
        bail!(
            "current branch is already attached to request {request_id}; run scope request update {request_id} instead"
        );
    }
    Ok(())
}

fn ensure_request_branch_context(git_repo: &GitRepo, command_name: &str) -> anyhow::Result<()> {
    let branch = current_branch(git_repo)?;
    if branch_config_value(git_repo, &branch, REQUEST_REMOTE_KEY)?.is_some()
        || branch_config_value(git_repo, &branch, REQUEST_ID_KEY)?.is_some()
    {
        return Ok(());
    }
    bail!("{command_name} requires a Scope request branch; run scope request start first")
}

fn store_branch_context(
    git_repo: &GitRepo,
    branch: &str,
    context: &RequestContext,
) -> anyhow::Result<()> {
    set_branch_config_value(git_repo, branch, REQUEST_OWNER_KEY, &context.target.owner)?;
    set_branch_config_value(git_repo, branch, REQUEST_REPO_KEY, &context.target.repo)?;
    set_branch_config_value(git_repo, branch, REQUEST_REMOTE_KEY, &context.target.remote)?;
    set_branch_config_value(
        git_repo,
        branch,
        REQUEST_BASE_AUDIENCE_KEY,
        base_audience_value_for_repo(&context.repo),
    )?;
    Ok(())
}

fn store_request_metadata(
    git_repo: &GitRepo,
    branch: &str,
    context: &RequestContext,
    request: &RequestSummaryResponse,
) -> anyhow::Result<()> {
    store_branch_context(git_repo, branch, context)?;
    set_branch_config_value(git_repo, branch, REQUEST_ID_KEY, &request.id)?;
    set_branch_config_value(git_repo, branch, REQUEST_REF_KEY, &request.request_ref)?;
    Ok(())
}

fn print_change_summary(
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

fn projection_label_for_repo(repo: &RepoSummaryResponse) -> &'static str {
    match repo.access.actor {
        RepositoryActor::Owner | RepositoryActor::Member => "private main",
        RepositoryActor::Public => "public main",
    }
}

fn base_audience_value_for_repo(repo: &RepoSummaryResponse) -> &'static str {
    base_audience_config_value(base_audience_for_repo(repo))
}

fn base_audience_for_repo(repo: &RepoSummaryResponse) -> RequestBaseAudience {
    match repo.access.actor {
        RepositoryActor::Owner | RepositoryActor::Member => RequestBaseAudience::Private,
        RepositoryActor::Public => RequestBaseAudience::Public,
    }
}

fn base_audience_config_value(audience: RequestBaseAudience) -> &'static str {
    match audience {
        RequestBaseAudience::Public => "public",
        RequestBaseAudience::Private => "private",
    }
}

fn parse_base_audience_config(value: &str) -> anyhow::Result<RequestBaseAudience> {
    match value {
        "public" => Ok(RequestBaseAudience::Public),
        "private" => Ok(RequestBaseAudience::Private),
        _ => bail!("invalid Scope request base audience '{value}'"),
    }
}

fn remote_main_ref(remote: &str) -> String {
    format!("refs/remotes/{remote}/{DEFAULT_SCOPE_BRANCH}")
}

fn normalized_optional_arg(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn default_request_branch_name() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("scope/request/{now}")
}
