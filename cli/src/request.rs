use crate::{
    api::{
        MergeRequestParams, ResolveRequestParams, StartRequestParams, SubmitRequestParams,
        comment_request, delete_request as api_delete_request, get_request, list_requests,
        mark_request_needs_response, merge_request, resolve_request, respond_to_request,
        start_request as api_start_request, submit_request,
    },
    git_repo::{
        GitRepo, current_branch, ensure_clean_working_tree, ensure_git_repo_ready, head_oid,
        run_git_in_repo, scope_remote_head_oid, try_run_git_in_repo, warn_if_dirty_working_tree,
    },
    push::DEFAULT_SCOPE_BRANCH,
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;

mod args;
mod local;
mod remote;
mod render;
#[cfg(test)]
mod tests;
mod text;
pub use args::RequestArgs;
use args::{
    RequestAudienceArg, RequestCommand, RequestMergeArgs, RequestResolveArgs, RequestStartArgs,
};
use local::{
    fetch_main_projection, load_context, load_context_and_request_id,
    maybe_request_branch_audience, maybe_request_id_for_context, normalized_submit_stake,
    print_change_summary, projection_label_for_audience, push_request_head, remote_main_ref,
    request_id_for_context, store_request_metadata, track_request_branch_ref,
};
use render::{
    confirm_merge, ensure_mergeable, print_mutation_receipt, print_repo_access,
    print_request_detail, print_submit_stake, request_line,
};
use text::short_oid;

pub struct PreparedRequestCommand {
    args: RequestArgs,
    git_repo: GitRepo,
}

pub fn prepare_request_command(args: RequestArgs) -> anyhow::Result<PreparedRequestCommand> {
    let (command_name, needs_clean_tree) = match &args.command {
        RequestCommand::Start(_) => ("scope request start", true),
        RequestCommand::Submit(_) => ("scope request submit", false),
        RequestCommand::Push(_) => ("scope request push", false),
        RequestCommand::Delete(_) => ("scope request delete", false),
        RequestCommand::Status(_) => ("scope request status", false),
        RequestCommand::Comment(_) => ("scope request comment", false),
        RequestCommand::NeedsResponse(_) => ("scope request needs-response", false),
        RequestCommand::Respond(_) => ("scope request respond", false),
        RequestCommand::Resolve(_) => ("scope request resolve", false),
        RequestCommand::Merge(_) => ("scope request merge", false),
    };
    let git_repo = ensure_git_repo_ready(command_name)?;
    if needs_clean_tree {
        ensure_clean_working_tree(&git_repo, command_name)?;
    }
    Ok(PreparedRequestCommand { args, git_repo })
}

pub fn run_request_command(
    command: PreparedRequestCommand,
    client: &Client,
    api_url: &str,
    session_token: &str,
) -> anyhow::Result<()> {
    let PreparedRequestCommand { args, git_repo } = command;
    match args.command {
        RequestCommand::Start(args) => {
            start_request_branch(&git_repo, client, api_url, session_token, args)
        }
        RequestCommand::Submit(args) => submit_request_branch(
            &git_repo,
            client,
            api_url,
            session_token,
            args.remote,
            args.stake_credits,
        ),
        RequestCommand::Push(args) => push_request_branch(
            &git_repo,
            client,
            api_url,
            session_token,
            args.remote,
            args.request,
        ),
        RequestCommand::Delete(args) => delete_request_branch(
            &git_repo,
            client,
            api_url,
            session_token,
            args.remote,
            args.request,
        ),
        RequestCommand::Status(args) => show_request_status(
            &git_repo,
            client,
            api_url,
            session_token,
            args.remote,
            args.request,
        ),
        RequestCommand::Comment(args) => comment_on_request(
            &git_repo,
            client,
            api_url,
            session_token,
            args.remote,
            args.request,
            args.body,
        ),
        RequestCommand::NeedsResponse(args) => mark_needs_response(
            &git_repo,
            client,
            api_url,
            session_token,
            args.remote,
            args.request,
            args.body,
        ),
        RequestCommand::Respond(args) => respond_to_request_thread(
            &git_repo,
            client,
            api_url,
            session_token,
            args.remote,
            args.request,
            args.body,
        ),
        RequestCommand::Resolve(args) => {
            resolve_request_thread(&git_repo, client, api_url, session_token, args)
        }
        RequestCommand::Merge(args) => {
            merge_request_thread(&git_repo, client, api_url, session_token, args)
        }
    }
}

fn start_request_branch(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    args: RequestStartArgs,
) -> anyhow::Result<()> {
    let context = load_context(
        git_repo,
        client,
        api_url,
        session_token,
        args.remote.as_deref(),
    )?;
    print_repo_access(&context.repo);
    let audience = start_audience(&context.repo, args.audience)?;
    fetch_main_projection(git_repo, &context, audience, session_token)?;
    let branch = args.name.trim().to_string();
    scope_core::domain::requests::validate_request_name(&branch)
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let local_ref = format!("refs/heads/{branch}");
    if try_run_git_in_repo(git_repo, &["show-ref", "--verify", "--quiet", &local_ref])? {
        bail!("local branch '{branch}' already exists");
    }
    let remote_main = remote_main_ref(&context.target.remote);
    let base_oid = scope_remote_head_oid(git_repo, &context.target.remote, DEFAULT_SCOPE_BRANCH)?
        .context("Scope main projection did not produce a local remote ref")?;
    let response = api_start_request(
        client,
        api_url,
        session_token,
        StartRequestParams {
            owner: &context.target.owner,
            repo: &context.target.repo,
            name: branch.clone(),
            title: args.title,
            audience,
        },
    )?;
    if let Err(switch_error) = run_git_in_repo(
        git_repo,
        &["switch", "--no-track", "-c", &branch, &remote_main],
    ) {
        let cleanup = api_delete_request(
            client,
            api_url,
            session_token,
            &context.target.owner,
            &context.target.repo,
            &response.request.id,
        );
        return match cleanup {
            Ok(_) => Err(switch_error).context(
                "create local request branch failed; the empty request was deleted, so it is safe to retry",
            ),
            Err(cleanup_error) => Err(switch_error).context(format!(
                "create local request branch failed and cleanup also failed ({cleanup_error}); run `scope request delete {branch}` before retrying"
            )),
        };
    }
    store_request_metadata(git_repo, &branch, &context, &response.request)?;
    let request_head_oid = head_oid(git_repo)?;
    push_request_head(
        &context.target,
        session_token,
        &request_head_oid,
        &response.request.id,
        &response.request.name,
    )?;
    track_request_branch_ref(
        git_repo,
        &branch,
        &context.target,
        &response.request.name,
        &request_head_oid,
    )?;

    println!(
        "Started request {} ({}) on branch {branch} from {} ({})",
        response.request.name,
        response.request.id,
        projection_label_for_audience(audience),
        short_oid(&base_oid)
    );
    println!("Next: commit changes, then run scope request push or scope request submit");
    println!(
        "Remote: {}/{}",
        context.target.remote, response.request.name
    );
    println!("Useful while working: scope pull, scope request status");
    Ok(())
}

fn submit_request_branch(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    stake_credits: Option<u32>,
) -> anyhow::Result<()> {
    warn_if_dirty_working_tree(git_repo)?;
    let context = load_context(git_repo, client, api_url, session_token, remote.as_deref())?;
    print_repo_access(&context.repo);
    let request_id =
        request_id_for_context(git_repo, client, api_url, session_token, &context, None)?;
    let detail = get_request(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )?;
    if !detail.request.permissions.can_push_branch {
        bail!(
            "request {} cannot be pushed by this user",
            detail.request.id
        );
    }
    let stake_credits = normalized_submit_stake(&context.repo, stake_credits)?;
    print_submit_stake(stake_credits);
    let audience = maybe_request_branch_audience(git_repo)?.unwrap_or(detail.request.audience);
    fetch_main_projection(git_repo, &context, audience, session_token)?;
    let request_head_oid = head_oid(git_repo)?;
    print_change_summary(git_repo, &context.target, &request_head_oid)?;

    push_request_head(
        &context.target,
        session_token,
        &request_head_oid,
        &detail.request.id,
        &detail.request.name,
    )
    .with_context(|| {
        format!(
            "request {} was not submitted because its branch was not pushed; retry scope request submit",
            detail.request.id
        )
    })?;
    let branch = current_branch(git_repo)?;
    track_request_branch_ref(
        git_repo,
        &branch,
        &context.target,
        &detail.request.name,
        &request_head_oid,
    )?;
    let response = submit_request(
        client,
        api_url,
        session_token,
        SubmitRequestParams {
            owner: &context.target.owner,
            repo: &context.target.repo,
            request_id: &detail.request.id,
            head_oid: request_head_oid.clone(),
            stake_credits,
        },
    )?;
    store_request_metadata(git_repo, &branch, &context, &response.request)?;

    println!(
        "Submitted request {} at {}/{}",
        response.request.id, context.target.remote, response.request.name
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

fn push_request_branch(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
) -> anyhow::Result<()> {
    warn_if_dirty_working_tree(git_repo)?;
    let context = load_context(git_repo, client, api_url, session_token, remote.as_deref())?;
    print_repo_access(&context.repo);
    let request_id = request_id_for_context(
        git_repo,
        client,
        api_url,
        session_token,
        &context,
        request_id,
    )?;
    let detail = get_request(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )?;
    if !detail.request.permissions.can_push_branch {
        bail!(
            "request {} cannot be pushed by this user",
            detail.request.id
        );
    }
    let request_head_oid = head_oid(git_repo)?;
    push_request_head(
        &context.target,
        session_token,
        &request_head_oid,
        &detail.request.id,
        &detail.request.name,
    )?;
    let branch = current_branch(git_repo)?;
    track_request_branch_ref(
        git_repo,
        &branch,
        &context.target,
        &detail.request.name,
        &request_head_oid,
    )?;
    store_request_metadata(git_repo, &branch, &context, &detail.request)?;
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

fn show_request_status(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
) -> anyhow::Result<()> {
    let context = load_context(git_repo, client, api_url, session_token, remote.as_deref())?;
    print_repo_access(&context.repo);
    if let Some(request_id) = maybe_request_id_for_context(
        git_repo,
        client,
        api_url,
        session_token,
        &context,
        request_id,
    )? {
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
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
    body: String,
) -> anyhow::Result<()> {
    let (context, request_id) =
        load_context_and_request_id(git_repo, client, api_url, session_token, remote, request_id)?;
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
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
    body: String,
) -> anyhow::Result<()> {
    let (context, request_id) =
        load_context_and_request_id(git_repo, client, api_url, session_token, remote, request_id)?;
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
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
    body: Option<String>,
) -> anyhow::Result<()> {
    let (context, request_id) =
        load_context_and_request_id(git_repo, client, api_url, session_token, remote, request_id)?;
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
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    args: RequestResolveArgs,
) -> anyhow::Result<()> {
    let (context, request_id) = load_context_and_request_id(
        git_repo,
        client,
        api_url,
        session_token,
        args.remote,
        args.request,
    )?;
    let response = resolve_request(
        client,
        api_url,
        session_token,
        ResolveRequestParams {
            owner: &context.target.owner,
            repo: &context.target.repo,
            request_id: &request_id,
            disposition: args.disposition.into(),
            body: args.body,
        },
    )?;
    print_mutation_receipt("Request resolved", &response);
    Ok(())
}

fn merge_request_thread(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    args: RequestMergeArgs,
) -> anyhow::Result<()> {
    let (context, request_id) = load_context_and_request_id(
        git_repo,
        client,
        api_url,
        session_token,
        args.remote,
        args.request,
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
    if !args.yes {
        confirm_merge(&detail.request)?;
    }
    let expected_main_oid = detail
        .request
        .mergeability
        .current_main_oid
        .as_ref()
        .context("request has no current main oid to merge into")?;
    let response = merge_request(
        client,
        api_url,
        session_token,
        MergeRequestParams {
            owner: &context.target.owner,
            repo: &context.target.repo,
            request_id: &request_id,
            expected_main_oid: expected_main_oid.to_string(),
            expected_head_oid: detail.request.mergeability.request_head_oid.to_string(),
            body: args.body,
        },
    )?;
    print_mutation_receipt("Request merged", &response);
    Ok(())
}

fn delete_request_branch(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
) -> anyhow::Result<()> {
    let context = load_context(git_repo, client, api_url, session_token, remote.as_deref())?;
    print_repo_access(&context.repo);
    let request_id = request_id_for_context(
        git_repo,
        client,
        api_url,
        session_token,
        &context,
        request_id,
    )?;
    let response = api_delete_request(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )?;
    if response.deleted {
        println!("Deleted working request {request_id}");
    } else if let Some(request) = response.request {
        println!("Withdrew request {}", request.id);
    }
    Ok(())
}

fn start_audience(
    repo: &crate::api::RepoSummaryResponse,
    requested: Option<RequestAudienceArg>,
) -> anyhow::Result<scope_core::domain::requests::RequestAudience> {
    use crate::api::RepositoryActor;
    use scope_core::domain::requests::RequestAudience;

    match repo.access.actor {
        RepositoryActor::Public => match requested.map(Into::into) {
            None | Some(RequestAudience::Public) => Ok(RequestAudience::Public),
            Some(RequestAudience::Private) => {
                bail!("public contributors can only start public requests")
            }
        },
        RepositoryActor::Owner | RepositoryActor::Member => requested
            .map(Into::into)
            .context("maintainers must choose --audience public or --audience private"),
    }
}
