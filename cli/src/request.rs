use crate::{
    api::{
        CreateRequestDiscussionParams, CreateRequestDiscussionReplyParams, RequestActivityParams,
        RequestTarget, StartRequestParams, add_request_invitee, close_request as api_close_request,
        create_request_discussion, create_request_discussion_reply, edit_request_identity,
        get_request, get_request_activity, leave_request, list_requests, merge_request,
        rate_request, remove_request_invitee, reopen_and_reply_to_request_discussion,
        resolve_request_discussion, start_request as api_start_request,
        submit_request as api_submit_request,
    },
    git_repo::{
        GitRepo, current_branch, ensure_clean_working_tree, ensure_git_repo_ready, head_oid,
        request_side_changed_file_paths, run_git_in_repo, try_run_git_in_repo,
        warn_if_dirty_working_tree,
    },
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_api_contract::{ErrorCode, ErrorResponse, RequestAudience, RequestDiscussionAnchor};
use scope_domain::{policy::ScopePath, repo_control::is_public_request_protected_path};
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

mod actions;
mod args;
mod confirm;
mod local;
mod outcome;
mod remote;
mod render;
#[cfg(test)]
mod tests;
mod text;
use actions::*;
pub use args::RequestArgs;
use args::{
    RequestAudienceArg, RequestCommand, RequestDiscussionArgs, RequestDiscussionCommand,
    RequestDiscussionReopenArgs, RequestDiscussionReplyArgs, RequestDiscussionResolveArgs,
    RequestDiscussionStartArgs, RequestStartArgs, RequestTargetArgs,
};
use confirm::require_confirmation;
use local::{
    load_context, load_context_and_request_id, maybe_request_id_for_context,
    projection_label_for_audience, push_request_head, refresh_main_projection, remote_main_ref,
    request_id_for_context, store_request_metadata, track_request_branch_ref,
};
use outcome::*;
use render::{
    close_receipt, discussion_reopened_receipt, discussion_replied_receipt,
    discussion_resolved_receipt, discussion_started_receipt, invitee_added_receipt,
    invitee_removed_receipt, leave_receipt, repo_access_lines, request_activity_lines_for_response,
    request_detail_lines_for_response, request_list_line, request_mutation_receipt_lines,
};
use text::{discussion_body, short_oid};

static CLIENT_DISCUSSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static CLIENT_REPLY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct PreparedRequestCommand {
    args: RequestArgs,
    git_repo: GitRepo,
}

fn discussion_command_name(command: &RequestDiscussionCommand) -> &'static str {
    match command {
        RequestDiscussionCommand::Start(_) => "scope request discussion start",
        RequestDiscussionCommand::Reply(_) => "scope request discussion reply",
        RequestDiscussionCommand::Resolve(_) => "scope request discussion resolve",
        RequestDiscussionCommand::Reopen(_) => "scope request discussion reopen",
    }
}

pub fn prepare_request_command(args: RequestArgs) -> anyhow::Result<PreparedRequestCommand> {
    let (command_name, needs_clean_tree) = match &args.command {
        RequestCommand::Start(_) => ("scope request start", true),
        RequestCommand::Push(_) => ("scope request push", false),
        RequestCommand::Submit(_) => ("scope request submit", false),
        RequestCommand::Close(_) => ("scope request close", false),
        RequestCommand::Edit(_) => ("scope request edit", false),
        RequestCommand::Invite(_) => ("scope request invite", false),
        RequestCommand::Uninvite(_) => ("scope request uninvite", false),
        RequestCommand::Leave(_) => ("scope request leave", false),
        RequestCommand::Merge(_) => ("scope request merge", false),
        RequestCommand::Rate(_) => ("scope request rate", false),
        RequestCommand::Discussion(args) => (discussion_command_name(&args.command), false),
        RequestCommand::Show(_) => ("scope request show", false),
        RequestCommand::List(_) => ("scope request list", false),
        RequestCommand::Status(_) => ("scope request status", false),
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
    machine_output: bool,
) -> anyhow::Result<RequestCommandOutcome> {
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
            args.target.remote,
            args.target.request,
            machine_output,
        ),
        RequestCommand::Submit(args) => submit_request_command(
            &git_repo,
            client,
            api_url,
            session_token,
            args.target,
            args.yes,
            machine_output,
        ),
        RequestCommand::Close(args) => close_request_branch(
            &git_repo,
            client,
            api_url,
            session_token,
            args.target,
            args.yes,
            machine_output,
        ),
        RequestCommand::Edit(args) => edit_request(
            &git_repo,
            client,
            api_url,
            session_token,
            args.target,
            args.title,
            args.description_file,
        ),
        RequestCommand::Invite(args) => invite_request(
            &git_repo,
            client,
            api_url,
            session_token,
            args.target,
            args.handle,
            true,
        ),
        RequestCommand::Uninvite(args) => invite_request(
            &git_repo,
            client,
            api_url,
            session_token,
            args.target,
            args.handle,
            false,
        ),
        RequestCommand::Leave(args) => {
            leave_invited_request(&git_repo, client, api_url, session_token, args.target)
        }
        RequestCommand::Merge(args) => merge_request_command(
            &git_repo,
            client,
            api_url,
            session_token,
            args.target,
            args.yes,
            machine_output,
        ),
        RequestCommand::Rate(args) => rate_request_command(
            &git_repo,
            client,
            api_url,
            session_token,
            args.target,
            args.score,
            args.reason,
        ),
        RequestCommand::Discussion(args) => {
            run_request_discussion_command(&git_repo, client, api_url, session_token, args)
        }
        RequestCommand::Show(args) => {
            show_one_request(&git_repo, client, api_url, session_token, args.target)
        }
        RequestCommand::List(args) => {
            list_request_status(&git_repo, client, api_url, session_token, args.remote)
        }
        RequestCommand::Status(args) => show_request_status(
            &git_repo,
            client,
            api_url,
            session_token,
            args.target.remote,
            args.target.request,
        ),
    }
}

fn start_request_branch(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    args: RequestStartArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let context = load_context(
        git_repo,
        client,
        api_url,
        session_token,
        args.remote.as_deref(),
    )?;
    let audience = start_audience(context.repo.access.actor, args.audience)?;
    let base_oid = refresh_main_projection(git_repo, &context.target, audience, session_token)?;
    let branch = args.name.trim().to_string();
    scope_domain::requests::validate_request_name(&branch)
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let local_ref = format!("refs/heads/{branch}");
    if try_run_git_in_repo(git_repo, &["show-ref", "--verify", "--quiet", &local_ref])? {
        bail!("local branch '{branch}' already exists");
    }
    let remote_main = remote_main_ref(&context.target.remote);
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
        &[
            "switch",
            "--quiet",
            "--no-track",
            "-c",
            &branch,
            &remote_main,
        ],
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

    let mut human_lines = repo_access_lines(&context.repo);
    human_lines.extend([
        format!(
            "Started request {} ({}) on branch {branch} from {} ({})",
            response.request.name,
            response.request.id,
            projection_label_for_audience(audience),
            short_oid(&base_oid)
        ),
        "Next: commit changes, then run scope request push".to_string(),
        format!(
            "Remote: {}/{}",
            context.target.remote, response.request.name
        ),
        "Useful while working: scope pull, scope request status".to_string(),
    ]);
    let result = StartResult {
        repo: context.repo,
        request: response.request,
        branch,
        base_oid,
        remote: context.target.remote,
    };
    Ok(RequestCommandOutcome::new(
        "request.start",
        RequestCommandResult::Started(result),
        human_lines,
    ))
}

fn push_request_branch(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
    machine_output: bool,
) -> anyhow::Result<RequestCommandOutcome> {
    if !machine_output {
        warn_if_dirty_working_tree(git_repo)?;
    }
    let context = load_context(git_repo, client, api_url, session_token, remote.as_deref())?;
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
        return Err(crate::error::CliError::new(ErrorResponse::new(
            ErrorCode::Forbidden,
            format!(
                "request {} cannot be pushed by this user",
                detail.request.id
            ),
        ))
        .into());
    }
    let request_head_oid = head_oid(git_repo)?;
    let current_main_oid = refresh_main_projection(
        git_repo,
        &context.target,
        detail.request.audience,
        session_token,
    )?;
    ensure_public_request_paths_allowed(git_repo, &detail, &current_main_oid, &request_head_oid)?;
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
    let mut human_lines = repo_access_lines(&context.repo);
    human_lines.extend(request_detail_lines_for_response(&detail));
    let result = DetailResult {
        repo: context.repo,
        request: detail.request,
        activity: None,
    };
    Ok(RequestCommandOutcome::new(
        "request.push",
        RequestCommandResult::Detail(result),
        human_lines,
    ))
}

fn ensure_public_request_paths_allowed(
    git_repo: &GitRepo,
    detail: &crate::api::RequestDetailResponse,
    current_main_oid: &str,
    request_head_oid: &str,
) -> anyhow::Result<()> {
    if detail.request.audience != RequestAudience::Public {
        return Ok(());
    }
    let changed_paths = request_side_changed_file_paths(
        git_repo,
        detail.request.base_main_oid.as_str(),
        current_main_oid,
        request_head_oid,
    )?;
    let protected_paths = changed_paths
        .into_iter()
        .filter_map(|path| {
            let scope_path = ScopePath::parse(format!("/{path}")).ok()?;
            is_public_request_protected_path(&scope_path).then_some(path)
        })
        .collect::<Vec<_>>();
    if protected_paths.is_empty() {
        return Ok(());
    }

    let message = format!(
        "public request cannot change maintainer-controlled paths: {}",
        protected_paths.join(", ")
    );
    let response = ErrorResponse::new(ErrorCode::ProtectedPath, message)
        .with_paths(protected_paths)
        .with_instruction(
            "Move maintainer-controlled changes to a maintainer-authored change, then retry.",
        );
    Err(crate::error::CliError::new(response).into())
}

fn show_request_status(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
) -> anyhow::Result<RequestCommandOutcome> {
    let context = load_context(git_repo, client, api_url, session_token, remote.as_deref())?;
    let mut human_lines = repo_access_lines(&context.repo);
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
        human_lines.extend(request_detail_lines_for_response(&detail));
        return Ok(RequestCommandOutcome::new(
            "request.status",
            RequestCommandResult::Detail(DetailResult {
                repo: context.repo,
                request: detail.request,
                activity: None,
            }),
            human_lines,
        ));
    }

    let requests = load_request_list(client, api_url, session_token, &context)?;
    human_lines.extend(request_list_lines(&requests)?);
    Ok(RequestCommandOutcome::new(
        "request.status",
        RequestCommandResult::List(ListResult {
            repo: context.repo,
            requests,
        }),
        human_lines,
    ))
}

fn run_request_discussion_command(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    args: RequestDiscussionArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    match args.command {
        RequestDiscussionCommand::Start(args) => {
            start_request_discussion(git_repo, client, api_url, session_token, args)
        }
        RequestDiscussionCommand::Reply(args) => {
            reply_to_request_discussion(git_repo, client, api_url, session_token, args)
        }
        RequestDiscussionCommand::Resolve(args) => {
            resolve_one_request_discussion(git_repo, client, api_url, session_token, args)
        }
        RequestDiscussionCommand::Reopen(args) => {
            reopen_request_discussion(git_repo, client, api_url, session_token, args)
        }
    }
}

fn start_request_discussion(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    args: RequestDiscussionStartArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let body = discussion_body(args.content.body, args.content.body_file)?;
    let (context, request_id) = load_context_and_request_id(
        git_repo,
        client,
        api_url,
        session_token,
        args.target.remote,
        args.target.request,
    )?;
    let anchor = args.revision.map(|revision_id| RequestDiscussionAnchor {
        revision_id,
        commit_oid: args.commit,
        path: args.path,
    });
    let response = create_request_discussion(
        client,
        api_url,
        session_token,
        CreateRequestDiscussionParams {
            target: RequestTarget {
                owner: &context.target.owner,
                repo: &context.target.repo,
                request_id: &request_id,
            },
            body_markdown: body,
            client_discussion_id: new_client_discussion_id()?,
            anchor,
        },
    )?;
    let human_lines = discussion_started_receipt(&request_id, &response);
    Ok(RequestCommandOutcome::new(
        "request.discussion.start",
        RequestCommandResult::Discussion(DiscussionResult {
            repo: context.repo,
            request_id,
            discussion: response.discussion,
        }),
        human_lines,
    ))
}

fn reply_to_request_discussion(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    args: RequestDiscussionReplyArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let body = discussion_body(args.content.body, args.content.body_file)?;
    let (context, request_id) = load_context_and_request_id(
        git_repo,
        client,
        api_url,
        session_token,
        args.target.remote,
        args.target.request,
    )?;
    let response = create_request_discussion_reply(
        client,
        api_url,
        session_token,
        CreateRequestDiscussionReplyParams {
            target: RequestTarget {
                owner: &context.target.owner,
                repo: &context.target.repo,
                request_id: &request_id,
            },
            discussion_id: &args.discussion_id,
            body_markdown: body,
            client_reply_id: new_client_reply_id()?,
        },
    )?;
    let human_lines = vec![discussion_replied_receipt(&response)];
    Ok(RequestCommandOutcome::new(
        "request.discussion.reply",
        RequestCommandResult::DiscussionReply(DiscussionReplyResult {
            repo: context.repo,
            request_id,
            discussion: response.discussion,
            reply: response.reply,
        }),
        human_lines,
    ))
}

fn resolve_one_request_discussion(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    args: RequestDiscussionResolveArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id) = load_context_and_request_id(
        git_repo,
        client,
        api_url,
        session_token,
        args.target.remote,
        args.target.request,
    )?;
    let response = resolve_request_discussion(
        client,
        api_url,
        session_token,
        RequestTarget {
            owner: &context.target.owner,
            repo: &context.target.repo,
            request_id: &request_id,
        },
        &args.discussion_id,
    )?;
    let human_lines = vec![discussion_resolved_receipt(&response)];
    Ok(RequestCommandOutcome::new(
        "request.discussion.resolve",
        RequestCommandResult::Discussion(DiscussionResult {
            repo: context.repo,
            request_id,
            discussion: response.discussion,
        }),
        human_lines,
    ))
}

fn reopen_request_discussion(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    args: RequestDiscussionReopenArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let body = discussion_body(args.content.body, args.content.body_file)?;
    let (context, request_id) = load_context_and_request_id(
        git_repo,
        client,
        api_url,
        session_token,
        args.target.remote,
        args.target.request,
    )?;
    let response = reopen_and_reply_to_request_discussion(
        client,
        api_url,
        session_token,
        CreateRequestDiscussionReplyParams {
            target: RequestTarget {
                owner: &context.target.owner,
                repo: &context.target.repo,
                request_id: &request_id,
            },
            discussion_id: &args.discussion_id,
            body_markdown: body,
            client_reply_id: new_client_reply_id()?,
        },
    )?;
    let human_lines = vec![discussion_reopened_receipt(&response)];
    Ok(RequestCommandOutcome::new(
        "request.discussion.reopen",
        RequestCommandResult::DiscussionReply(DiscussionReplyResult {
            repo: context.repo,
            request_id,
            discussion: response.discussion,
            reply: response.reply,
        }),
        human_lines,
    ))
}

fn new_client_discussion_id() -> anyhow::Result<String> {
    new_client_mutation_id("discussion", &CLIENT_DISCUSSION_SEQUENCE)
}

fn new_client_reply_id() -> anyhow::Result<String> {
    new_client_mutation_id("reply", &CLIENT_REPLY_SEQUENCE)
}

fn new_client_mutation_id(kind: &str, sequence: &AtomicU64) -> anyhow::Result<String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_nanos();
    Ok(format!(
        "client_{kind}_{}_{}_{}",
        std::process::id(),
        nanos,
        sequence.fetch_add(1, Ordering::Relaxed)
    ))
}

fn close_request_branch(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
    yes: bool,
    machine_output: bool,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, before) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    let prompt = if before.request.submitted_at_unix.is_none() {
        format!("Permanently delete draft request {}", before.request.name)
    } else {
        format!("Close published request {}", before.request.name)
    };
    require_confirmation(&prompt, yes, !machine_output)?;
    let response = api_close_request(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )?;
    let human_line = close_receipt(&request_id, &response);
    Ok(RequestCommandOutcome::new(
        "request.close",
        RequestCommandResult::Close(TargetResponse {
            repo: context.repo,
            request_id,
            response,
        }),
        vec![human_line],
    ))
}

fn start_audience(
    actor: crate::api::RepositoryActor,
    requested: Option<RequestAudienceArg>,
) -> anyhow::Result<crate::api::RequestAudience> {
    use crate::api::RepositoryActor;
    use crate::api::RequestAudience;

    match actor {
        RepositoryActor::Public => match requested.map(Into::into) {
            None | Some(RequestAudience::Public) => Ok(RequestAudience::Public),
            Some(RequestAudience::Private) => {
                bail!("public contributors can only start public requests")
            }
        },
        RepositoryActor::Owner | RepositoryActor::Member => Ok(requested
            .map(Into::into)
            .unwrap_or(RequestAudience::Private)),
    }
}

#[cfg(test)]
mod audience_tests {
    use super::*;
    use crate::api::{RepositoryActor, RequestAudience};

    #[test]
    fn maintainers_default_to_private_requests() {
        for actor in [RepositoryActor::Owner, RepositoryActor::Member] {
            assert_eq!(
                start_audience(actor, None).unwrap(),
                RequestAudience::Private
            );
        }
    }

    #[test]
    fn maintainers_can_explicitly_choose_request_audience() {
        for actor in [RepositoryActor::Owner, RepositoryActor::Member] {
            for (requested, expected) in [
                (RequestAudienceArg::Public, RequestAudience::Public),
                (RequestAudienceArg::Private, RequestAudience::Private),
            ] {
                assert_eq!(start_audience(actor, Some(requested)).unwrap(), expected);
            }
        }
    }

    #[test]
    fn public_contributors_default_to_public_requests() {
        assert_eq!(
            start_audience(RepositoryActor::Public, None).unwrap(),
            RequestAudience::Public
        );
    }

    #[test]
    fn public_contributors_can_only_choose_public_requests() {
        assert_eq!(
            start_audience(RepositoryActor::Public, Some(RequestAudienceArg::Public)).unwrap(),
            RequestAudience::Public
        );
        assert_eq!(
            start_audience(RepositoryActor::Public, Some(RequestAudienceArg::Private))
                .unwrap_err()
                .to_string(),
            "public contributors can only start public requests"
        );
    }
}
