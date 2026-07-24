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

pub(super) fn ready_request(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
    stake: u32,
    yes: bool,
) -> anyhow::Result<()> {
    let (context, request_id, before) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    let uses_credits =
        before.request.author_role == scope_core::domain::requests::RequestActorRole::Public;
    let prompt = match (before.request.first_ready_at_unix.is_none(), uses_credits) {
        (true, true) => format!("This publishes the request and holds {stake} credits"),
        (false, true) => format!("This holds {stake} credits and returns the request to review"),
        (true, false) => "This publishes the request for review".to_string(),
        (false, false) => "This returns the request to review".to_string(),
    };
    require_confirmation(&prompt, yes)?;
    let response = mark_request_ready(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
        uses_credits.then_some(stake),
    )?;
    print_request_mutation_receipt("Ready for review", Some(&before.request), &response);
    Ok(())
}

pub(super) fn working_request(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
) -> anyhow::Result<()> {
    let (context, request_id, before) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    let response = return_request_to_working(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
    )?;
    print_request_mutation_receipt("Returned to Working", Some(&before.request), &response);
    Ok(())
}

pub(super) fn edit_request(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
    title: Option<String>,
    description_file: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
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
    print_request_mutation_receipt("Edited request", Some(&before.request), &response);
    Ok(())
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
) -> anyhow::Result<()> {
    let (context, request_id, _) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    let handle = exact_handle(handle)?;
    if invite {
        let response = add_request_invitee(
            client,
            api_url,
            session_token,
            api_target(&context, &request_id),
            handle,
        )?;
        print_invitee_added_receipt(&response);
    } else {
        let response = remove_request_invitee(
            client,
            api_url,
            session_token,
            api_target(&context, &request_id),
            handle,
        )?;
        print_invitee_removed_receipt(&response);
    }
    Ok(())
}

pub(super) fn leave_invited_request(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
) -> anyhow::Result<()> {
    let (context, request_id, _) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    let response = leave_request(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
    )?;
    print_leave_receipt(&request_id, &response);
    Ok(())
}

pub(super) fn hold_request_command(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
    held: bool,
) -> anyhow::Result<()> {
    let (context, request_id, before) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    let target = api_target(&context, &request_id);
    let response = if held {
        hold_request(client, api_url, session_token, target)?
    } else {
        unhold_request(client, api_url, session_token, target)?
    };
    print_request_mutation_receipt(
        if held {
            "Review hold started"
        } else {
            "Review hold released"
        },
        Some(&before.request),
        &response,
    );
    Ok(())
}

pub(super) fn request_changes_command(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
) -> anyhow::Result<()> {
    let (context, request_id, before) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    let response = request_changes(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
    )?;
    print_request_mutation_receipt("Changes requested", Some(&before.request), &response);
    Ok(())
}

pub(super) fn assess_request_command(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    args: RequestAssessArgs,
) -> anyhow::Result<()> {
    let RequestAssessArgs {
        target,
        outcome,
        message,
        yes,
    } = args;
    let outcome = outcome.into();
    let (context, request_id, before) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    require_confirmation(
        &assessment_confirmation(
            &before.request.name,
            outcome,
            before.request.current_stake_credits,
        ),
        yes,
    )?;
    let response = assess_request(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
        outcome,
        message,
    )?;
    print_request_mutation_receipt("Request assessed", Some(&before.request), &response);
    print_committed_settlement(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
        before.request.activity_version,
        response.request.activity_version,
        "Assessment committed",
    );
    Ok(())
}

pub(super) fn merge_request_command(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTargetArgs,
    yes: bool,
) -> anyhow::Result<()> {
    let (context, request_id, before) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    require_confirmation(
        &merge_confirmation(&before.request.name, before.request.state),
        yes,
    )?;
    let response = merge_request(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
    )?;
    print_request_mutation_receipt("Merged", Some(&before.request), &response);
    print_committed_settlement(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
        before.request.activity_version,
        response.request.activity_version,
        "Merge committed",
    );
    Ok(())
}

fn assessment_confirmation(
    request_name: &str,
    outcome: crate::api::RequestAssessmentOutcome,
    stake_credits: u32,
) -> String {
    let settlement = if stake_credits > 0 {
        " and settle its review stake"
    } else {
        ""
    };
    format!("Complete request {request_name} as {outcome:?}{settlement}")
}

fn merge_confirmation(
    request_name: &str,
    state: scope_core::domain::requests::RequestState,
) -> String {
    if state == scope_core::domain::requests::RequestState::ReadyForReview {
        format!("Merge request {request_name} into main and complete it as Accepted")
    } else {
        format!("Merge accepted request {request_name} into main")
    }
}

fn print_committed_settlement(
    client: &Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
    after_position: u64,
    version: u64,
    committed_label: &str,
) {
    match full_request_activity(
        client,
        api_url,
        session_token,
        target,
        after_position,
        version,
    ) {
        Ok(activity) => print_request_settlement(&activity),
        Err(error) => {
            eprintln!("{committed_label}, but its settlement receipt could not be loaded: {error}")
        }
    }
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
) -> anyhow::Result<()> {
    let (context, request_id, detail) =
        load_exact_request(git_repo, client, api_url, session_token, target)?;
    print_request_detail(&detail);
    print_request_activity(&full_request_activity(
        client,
        api_url,
        session_token,
        api_target(&context, &request_id),
        0,
        detail.request.activity_version,
    )?);
    Ok(())
}

pub(super) fn list_request_status(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
) -> anyhow::Result<()> {
    let context = load_context(git_repo, client, api_url, session_token, remote.as_deref())?;
    print_repo_access(&context.repo);
    print_request_list(client, api_url, session_token, &context)
}

pub(super) fn print_request_list(
    client: &Client,
    api_url: &str,
    session_token: &str,
    context: &local::RequestContext,
) -> anyhow::Result<()> {
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
            scope_core::domain::requests::RequestState::ReadyForReview => 0,
            scope_core::domain::requests::RequestState::Working => 1,
            scope_core::domain::requests::RequestState::Completed => 2,
        };
        let state_order = rank(left.state).cmp(&rank(right.state));
        if state_order != std::cmp::Ordering::Equal {
            return state_order;
        }
        if left.state == scope_core::domain::requests::RequestState::ReadyForReview {
            return right
                .current_stake_credits
                .cmp(&left.current_stake_credits)
                .then_with(|| {
                    left.ready_at_unix.cmp(&right.ready_at_unix)
                })
                .then_with(|| left.id.cmp(&right.id));
        }
        left.updated_at_unix
            .cmp(&right.updated_at_unix)
            .then_with(|| left.id.cmp(&right.id))
    });
    if requests.is_empty() {
        println!("No visible requests.");
    } else {
        println!(" WAIT  STAKE  STATE      REQUEST");
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before Unix epoch")?
            .as_secs();
        for request in requests {
            println!("{}", request_list_line(&request, now_unix));
        }
    }
    Ok(())
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
                "kind": "ReadyForReview",
                "payload": {"ReadyForReview": {
                    "head_oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "stake_credits": 12
                }},
                "created_at_unix": 20
            },
            {
                "id": "event_3", "position": 3,
                "actor": {"id": "scope_usr_actor", "handle": "actor"},
                "kind": "ReadyForReview",
                "payload": {"ReadyForReview": {
                    "head_oid": "cccccccccccccccccccccccccccccccccccccccc",
                    "stake_credits": 15
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
    fn assessment_confirmation_mentions_only_real_settlement() {
        let outcome = crate::api::RequestAssessmentOutcome::Neutral;
        assert_eq!(
            assessment_confirmation("change", outcome, 0),
            "Complete request change as Neutral"
        );
        assert_eq!(
            assessment_confirmation("change", outcome, 12),
            "Complete request change as Neutral and settle its review stake"
        );
    }

    #[test]
    fn ready_merge_confirmation_discloses_accepted_completion() {
        use scope_core::domain::requests::RequestState;
        assert_eq!(
            merge_confirmation("change", RequestState::ReadyForReview),
            "Merge request change into main and complete it as Accepted"
        );
        assert_eq!(
            merge_confirmation("change", RequestState::Completed),
            "Merge accepted request change into main"
        );
    }
}
