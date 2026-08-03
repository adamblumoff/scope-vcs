use super::text::terminal_text;
use super::*;
pub(super) fn load_exact_request(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
) -> anyhow::Result<(
    local::RequestContext,
    String,
    crate::api::RequestDetailResponse,
)> {
    let context = load_context(
        git_repo,
        client,
        api_url,
        session_token,
        target.remote.as_deref(),
    )?;
    let request_id = request_id_for_context(
        git_repo,
        client,
        api_url,
        session_token,
        &context,
        target.request,
    )?;
    let detail = get_request(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )?;
    Ok((context, request_id, detail))
}

fn api_target<'a>(context: &'a local::RequestContext, request_id: &'a str) -> RequestTarget<'a> {
    RequestTarget {
        owner: &context.target.owner,
        repo: &context.target.repo,
        request_id,
    }
}

pub(super) fn submit_request_command(
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
    let prompt = "Submit this request to its maintainers";
    require_confirmation(prompt, yes, !machine_output)?;
    let response = api_submit_request(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
    )?;
    let human_lines = request_mutation_receipt_lines("Submitted", Some(&before.request), &response);
    Ok(RequestCommandOutcome::new(
        "request.submit",
        RequestCommandResult::Mutation(RepoResponse {
            repo: context.repo,
            response,
        }),
        human_lines,
    ))
}

pub(super) fn edit_request(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
    title: Option<String>,
    description_file: Option<std::path::PathBuf>,
) -> anyhow::Result<RequestCommandOutcome> {
    let description = description_file
        .map(|path| {
            fs::read_to_string(&path)
                .with_context(|| format!("read request description from {}", path.display()))
        })
        .transpose()?;
    let (context, request_id, before) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    let response = edit_request_identity(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
        title,
        description,
    )?;
    let human_lines =
        request_mutation_receipt_lines("Edited request", Some(&before.request), &response);
    Ok(RequestCommandOutcome::new(
        "request.edit",
        RequestCommandResult::Mutation(RepoResponse {
            repo: context.repo,
            response,
        }),
        human_lines,
    ))
}

fn exact_handle(handle: String) -> anyhow::Result<String> {
    let handle = handle.trim().strip_prefix('@').unwrap_or(handle.trim());
    if handle.is_empty() {
        bail!("an exact Scope handle is required");
    }
    Ok(handle.to_string())
}

pub(super) fn invite_request(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
    handle: String,
    invite: bool,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, _) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    let handle = exact_handle(handle)?;
    let (command, response, human_line) = if invite {
        let response = add_request_invitee(
            client,
            api_url,
            session_token,
            api_target(&context, &request_id),
            handle,
        )?;
        let human_line = invitee_added_receipt(&response);
        ("request.invite", response, human_line)
    } else {
        let response = remove_request_invitee(
            client,
            api_url,
            session_token,
            api_target(&context, &request_id),
            handle,
        )?;
        let human_line = invitee_removed_receipt(&response);
        ("request.uninvite", response, human_line)
    };
    Ok(RequestCommandOutcome::new(
        command,
        RequestCommandResult::Invitee(RepoResponse {
            repo: context.repo,
            response,
        }),
        vec![human_line],
    ))
}

pub(super) fn leave_invited_request(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, _) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    let response = leave_request(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
    )?;
    let human_line = leave_receipt(&request_id, &response);
    Ok(RequestCommandOutcome::new(
        "request.leave",
        RequestCommandResult::Leave(TargetResponse {
            repo: context.repo,
            request_id,
            response,
        }),
        vec![human_line],
    ))
}

pub(super) fn merge_request_command(
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
    require_confirmation(
        &merge_confirmation(&before.request.name, before.request.state),
        yes,
        !machine_output,
    )?;
    let response = merge_request(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
    )?;
    let human_lines = request_mutation_receipt_lines("Merged", Some(&before.request), &response);
    Ok(RequestCommandOutcome::new(
        "request.merge",
        RequestCommandResult::Mutation(RepoResponse {
            repo: context.repo,
            response,
        }),
        human_lines,
    ))
}

pub(super) fn rate_request_command(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
    score: u8,
    reason: String,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, _) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    let response = rate_request(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
        score,
        reason,
    )?;
    let human_line = format!(
        "Rated @{} {}/5 — {}",
        terminal_text(&response.subject.handle),
        response.score,
        terminal_text(&response.reason)
    );
    Ok(RequestCommandOutcome::new(
        "request.rate",
        RequestCommandResult::Rating(TargetResponse {
            repo: context.repo,
            request_id,
            response,
        }),
        vec![human_line],
    ))
}

fn merge_confirmation(request_name: &str, _state: crate::api::RequestState) -> String {
    format!("Merge request {request_name} into main")
}

fn events_through_version(
    events: Vec<crate::api::RequestEventResponse>,
    version: u64,
) -> Vec<crate::api::RequestEventResponse> {
    events
        .into_iter()
        .filter(|event| event.position <= version)
        .collect()
}

fn full_request_activity(
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
    after_position: u64,
    version: u64,
) -> anyhow::Result<crate::api::RequestActivityPageResponse> {
    let mut events = Vec::new();
    let mut after = after_position;
    while after < version {
        let page = get_request_activity(
            client,
            api_url,
            session_token,
            RequestActivityParams {
                target,
                after: Some(after),
                latest: false,
                limit: Some(100),
            },
        )?;
        let page_events = events_through_version(page.events, version);
        let next = page_events
            .last()
            .map(|event| event.position)
            .unwrap_or(after);
        events.extend(page_events);
        if next == after {
            break;
        }
        after = next;
    }
    Ok(crate::api::RequestActivityPageResponse {
        events,
        through_position: version,
    })
}

pub(super) fn show_one_request(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, detail) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    let activity = full_request_activity(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
        0,
        detail.request.activity_version,
    )?;
    let mut human_lines = request_detail_lines_for_response(&detail);
    human_lines.extend(request_activity_lines_for_response(&activity));
    Ok(RequestCommandOutcome::new(
        "request.show",
        RequestCommandResult::Detail(DetailResult {
            repo: context.repo,
            request: detail.request,
            activity: Some(activity),
        }),
        human_lines,
    ))
}

pub(super) fn list_request_status(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
) -> anyhow::Result<RequestCommandOutcome> {
    let context = load_context(git_repo, client, api_url, session_token, remote.as_deref())?;
    let requests = load_request_list(client, api_url, session_token, &context)?;
    let mut human_lines = repo_access_lines(&context.repo);
    human_lines.extend(request_list_lines(&requests)?);
    Ok(RequestCommandOutcome::new(
        "request.list",
        RequestCommandResult::List(ListResult {
            repo: context.repo,
            requests,
        }),
        human_lines,
    ))
}

pub(super) fn load_request_list(
    client: &Client,
    api_url: &str,
    session_token: &str,
    context: &local::RequestContext,
) -> anyhow::Result<Vec<crate::api::RequestListItemResponse>> {
    let mut requests = Vec::new();
    let mut cursor = None;
    loop {
        let page = list_requests(
            client,
            api_url,
            session_token,
            &context.target.owner,
            &context.target.repo,
            cursor.as_deref(),
        )?;
        requests.extend(page.requests);
        let Some(next) = page.next_cursor else { break };
        cursor = Some(next);
    }
    requests.sort_by(|left, right| {
        let rank = |state| match state {
            crate::api::RequestState::Open => 0,
            crate::api::RequestState::Draft => 1,
            crate::api::RequestState::Closed => 2,
            crate::api::RequestState::Merged => 3,
        };
        let state_order = rank(left.state).cmp(&rank(right.state));
        if state_order != std::cmp::Ordering::Equal {
            return state_order;
        }
        if left.state == crate::api::RequestState::Open {
            return left
                .submitted_at_unix
                .cmp(&right.submitted_at_unix)
                .then_with(|| left.id.cmp(&right.id));
        }
        left.updated_at_unix
            .cmp(&right.updated_at_unix)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(requests)
}

pub(super) fn request_list_lines(
    requests: &[crate::api::RequestListItemResponse],
) -> anyhow::Result<Vec<String>> {
    if requests.is_empty() {
        return Ok(vec!["No visible requests.".to_string()]);
    }
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs();
    let mut lines = vec![" WAIT  STATE      REQUEST".to_string()];
    lines.extend(
        requests
            .iter()
            .map(|request| request_list_line(request, now_unix)),
    );
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn activity_events_are_bounded_to_the_response_version() {
        let events: Vec<crate::api::RequestEventResponse> = serde_json::from_value(json!([
            {
                "id": "event_2", "position": 2,
                "actor": {"id": "scope_usr_actor", "handle": "actor"},
                "kind": "Submitted",
                "payload": {"Submitted": {
                    "head_oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                }},
                "created_at_unix": 20
            },
            {
                "id": "event_3", "position": 3,
                "actor": {"id": "scope_usr_actor", "handle": "actor"},
                "kind": "Submitted",
                "payload": {"Submitted": {
                    "head_oid": "cccccccccccccccccccccccccccccccccccccccc"
                }},
                "created_at_unix": 30
            }
        ]))
        .unwrap();

        let bounded = events_through_version(events, 2);
        assert_eq!(bounded.len(), 1);
        assert_eq!(bounded[0].position, 2);
    }

    #[test]
    fn merge_confirmation_names_the_request() {
        use crate::api::RequestState;
        assert_eq!(
            merge_confirmation("change", RequestState::Open),
            "Merge request change into main"
        );
        assert_eq!(
            merge_confirmation("change", RequestState::Merged),
            "Merge request change into main"
        );
    }
}
