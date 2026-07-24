use crate::{
    api::{
        CreateRequestDiscussionParams, StartRequestParams, close_request as api_close_request,
        create_request_discussion, get_request, list_requests, start_request as api_start_request,
    },
    git_repo::{
        GitRepo, current_branch, ensure_clean_working_tree, ensure_git_repo_ready, head_oid,
        run_git_in_repo, scope_remote_head_oid, try_run_git_in_repo, warn_if_dirty_working_tree,
    },
    push::DEFAULT_SCOPE_BRANCH,
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

mod args;
mod local;
mod remote;
mod render;
#[cfg(test)]
mod tests;
mod text;
pub use args::RequestArgs;
use args::{RequestAudienceArg, RequestCommand, RequestStartArgs};
use local::{
    fetch_main_projection, load_context, load_context_and_request_id, maybe_request_id_for_context,
    projection_label_for_audience, push_request_head, remote_main_ref, request_id_for_context,
    store_request_metadata, track_request_branch_ref,
};
use render::{
    print_discussion_receipt, print_repo_access, print_request_detail, request_list_line,
};
use text::short_oid;

static CLIENT_DISCUSSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct PreparedRequestCommand {
    args: RequestArgs,
    git_repo: GitRepo,
}

pub fn prepare_request_command(args: RequestArgs) -> anyhow::Result<PreparedRequestCommand> {
    let (command_name, needs_clean_tree) = match &args.command {
        RequestCommand::Start(_) => ("scope request start", true),
        RequestCommand::Push(_) => ("scope request push", false),
        RequestCommand::Close(_) => ("scope request close", false),
        RequestCommand::Status(_) => ("scope request status", false),
        RequestCommand::Discuss(_) => ("scope request discuss", false),
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
        RequestCommand::Push(args) => push_request_branch(
            &git_repo,
            client,
            api_url,
            session_token,
            args.remote,
            args.request,
        ),
        RequestCommand::Close(args) => close_request_branch(
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
        RequestCommand::Discuss(args) => start_request_discussion(
            &git_repo,
            client,
            api_url,
            session_token,
            args.remote,
            args.request,
            args.body,
        ),
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
    let audience = start_audience(
        context.repo.access.actor,
        context.repo.default_visibility,
        args.audience,
    )?;
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
        let cleanup = api_close_request(
            client,
            api_url,
            session_token,
            &context.target.owner,
            &context.target.repo,
            &response.request.id,
        );
        return match cleanup {
            Ok(_) => Err(switch_error).context(
                "create local request branch failed; the empty request was closed and removed, so it is safe to retry",
            ),
            Err(cleanup_error) => Err(switch_error).context(format!(
                "create local request branch failed and cleanup also failed ({cleanup_error}); run `scope request close {branch}` before retrying"
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
    println!("Next: commit changes, then run scope request push");
    println!(
        "Remote: {}/{}",
        context.target.remote, response.request.name
    );
    println!("Useful while working: scope pull, scope request status");
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

    let mut cursor = None;
    let mut printed_heading = false;
    loop {
        let response = list_requests(
            client,
            api_url,
            session_token,
            &context.target.owner,
            &context.target.repo,
            cursor.as_deref(),
        )?;
        if !printed_heading && !response.requests.is_empty() {
            println!("Visible requests:");
            printed_heading = true;
        }
        for request in response.requests {
            println!("  {}", request_list_line(&request));
        }
        let Some(next_cursor) = response.next_cursor else {
            break;
        };
        cursor = Some(next_cursor);
    }
    if !printed_heading {
        println!("No visible requests.");
    }
    Ok(())
}

fn start_request_discussion(
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
    let response = create_request_discussion(
        client,
        api_url,
        session_token,
        CreateRequestDiscussionParams {
            owner: &context.target.owner,
            repo: &context.target.repo,
            request_id: &request_id,
            body_markdown: body,
            client_discussion_id: new_client_discussion_id()?,
        },
    )?;
    print_discussion_receipt(&response);
    Ok(())
}

fn new_client_discussion_id() -> anyhow::Result<String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_nanos();
    Ok(format!(
        "client_discussion_{}_{}_{}",
        std::process::id(),
        nanos,
        CLIENT_DISCUSSION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn close_request_branch(
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
    let response = api_close_request(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )?;
    if response.deleted {
        println!("Closed and removed unpublished Working request {request_id}");
    } else if let Some(request) = response.request {
        println!("Closed request {}", request.id);
    }
    Ok(())
}

fn start_audience(
    actor: crate::api::RepositoryActor,
    default_visibility: crate::api::Visibility,
    requested: Option<RequestAudienceArg>,
) -> anyhow::Result<scope_core::domain::requests::RequestAudience> {
    use crate::api::{RepositoryActor, Visibility};
    use scope_core::domain::requests::RequestAudience;

    match actor {
        RepositoryActor::Public => match requested.map(Into::into) {
            None | Some(RequestAudience::Public) => Ok(RequestAudience::Public),
            Some(RequestAudience::Private) => {
                bail!("public contributors can only start public requests")
            }
        },
        RepositoryActor::Owner | RepositoryActor::Member => Ok(requested
            .map(Into::into)
            .unwrap_or(match default_visibility {
                Visibility::Public => RequestAudience::Public,
                Visibility::Private => RequestAudience::Private,
            })),
    }
}

#[cfg(test)]
mod audience_tests {
    use super::*;
    use crate::api::{RepositoryActor, Visibility};
    use scope_core::domain::requests::RequestAudience;

    #[test]
    fn maintainers_default_request_audience_from_repo_visibility() {
        for (actor, visibility, expected) in [
            (
                RepositoryActor::Owner,
                Visibility::Public,
                RequestAudience::Public,
            ),
            (
                RepositoryActor::Owner,
                Visibility::Private,
                RequestAudience::Private,
            ),
            (
                RepositoryActor::Member,
                Visibility::Public,
                RequestAudience::Public,
            ),
            (
                RepositoryActor::Member,
                Visibility::Private,
                RequestAudience::Private,
            ),
        ] {
            assert_eq!(start_audience(actor, visibility, None).unwrap(), expected);
        }
    }

    #[test]
    fn explicit_maintainer_audience_overrides_repo_visibility() {
        assert_eq!(
            start_audience(
                RepositoryActor::Owner,
                Visibility::Private,
                Some(RequestAudienceArg::Public),
            )
            .unwrap(),
            RequestAudience::Public
        );
    }
}
