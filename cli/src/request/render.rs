use super::text::{short_oid, terminal_text};
use crate::api::{
    LeaveRequestResponse, RepoSummaryResponse, RepositoryActor, RequestActivityPageResponse,
    RequestAssessmentOutcome, RequestCloseResponse, RequestDetailResponse,
    RequestDiscussionMutationResponse, RequestEventPayload, RequestInviteeMutationResponse,
    RequestListItemResponse, RequestMergeabilityStatus, RequestMutationResponse,
    RequestPermissionsResponse, RequestSummaryResponse,
};
use scope_core::domain::requests::{
    RequestAudience, RequestDiscussionStatus, RequestReviewExitReason, RequestState,
};

pub(super) fn print_repo_access(repo: &RepoSummaryResponse) {
    println!("Scope repo: {}/{}", repo.owner_handle, repo.name);
    println!("Permission: {}", access_label(repo.access.actor));
    println!(
        "Review stake: {}",
        if repo.request_permissions.uses_credit_stake {
            "required when entering review"
        } else {
            "not used for owner/member requests"
        }
    );
}

pub(super) fn print_request_detail(detail: &RequestDetailResponse) {
    for line in request_detail_lines(&detail.request) {
        println!("{line}");
    }
}

pub(super) fn print_request_activity(activity: &RequestActivityPageResponse) {
    let lines = request_activity_lines(activity);
    if lines.is_empty() {
        return;
    }
    println!("Review cycles:");
    for line in lines {
        println!("  {line}");
    }
}

pub(super) fn print_request_settlement(activity: &RequestActivityPageResponse) {
    if let Some(line) = settlement_effect_line(activity) {
        println!("{line}");
    }
}

fn settlement_effect_line(activity: &RequestActivityPageResponse) -> Option<String> {
    let settlement = activity
        .events
        .iter()
        .filter_map(|event| match &event.payload {
            RequestEventPayload::Settled { settlement } => Some((event.position, settlement)),
            _ => None,
        })
        .max_by_key(|(position, _)| *position)
        .map(|(_, settlement)| settlement)?;
    (settlement.stake_credits > 0).then(|| {
        format!(
            "Credits: stake {} · refund {} · reward {} · burned {}",
            settlement.stake_credits,
            settlement.refunded_credits,
            settlement.reward_credits,
            settlement.burned_credits
        )
    })
}

pub(super) fn print_request_mutation_receipt(
    action: &str,
    before: Option<&RequestSummaryResponse>,
    response: &RequestMutationResponse,
) {
    let action = terminal_text(action);
    println!("{action} · {}", request_line(&response.request));
    if let Some(before) = before {
        for line in mutation_effect_lines(before, &response.request) {
            println!("{line}");
        }
    }
}

pub(super) fn print_invitee_added_receipt(response: &RequestInviteeMutationResponse) {
    println!(
        "Invited @{} · can now push request {}",
        terminal_text(&response.invitee.user.handle),
        response.request.name
    );
}

pub(super) fn print_invitee_removed_receipt(response: &RequestInviteeMutationResponse) {
    println!(
        "Removed @{} from request {}",
        terminal_text(&response.invitee.user.handle),
        response.request.name
    );
}

pub(super) fn print_leave_receipt(request_id: &str, response: &LeaveRequestResponse) {
    println!(
        "@{} left request {}",
        terminal_text(&response.invitee.user.handle),
        request_id
    );
}

pub(super) fn print_close_receipt(request_id: &str, response: &RequestCloseResponse) {
    if response.deleted {
        println!("Closed and removed unpublished Working request {request_id}");
    } else if let Some(request) = response.request.as_ref() {
        println!(
            "Closed request {} · Completed · remains in published history",
            request.name
        );
    } else {
        println!("Closed request {request_id}");
    }
}

pub(super) fn print_discussion_receipt(response: &RequestDiscussionMutationResponse) {
    let discussion = &response.discussion;
    println!(
        "Discussion opened: {} [{}] by @{}",
        discussion.id,
        discussion_status_label(discussion.status),
        terminal_text(&discussion.author.handle)
    );
    println!("Replies: {}", discussion.reply_count);
    println!(
        "{}",
        terminal_text(discussion.body_markdown.as_deref().unwrap_or("Code change"))
    );
}

pub(super) fn request_line(request: &RequestSummaryResponse) -> String {
    format_request_line(RequestLine {
        name: &request.name,
        id: &request.id,
        state: request.state,
        held: request.held_at_unix.is_some(),
        title: &request.title,
        stake_credits: request.current_stake_credits,
        head_oid: &request.head_oid,
        assessment_outcome: request.assessment_outcome,
    })
}

pub(super) fn request_list_line(request: &RequestListItemResponse, now_unix: u64) -> String {
    format!(
        "{:>5}  {}",
        wait_label(request.ready_at_unix, now_unix),
        format_request_line(RequestLine {
            name: &request.name,
            id: &request.id,
            state: request.state,
            held: request.held_at_unix.is_some(),
            title: &request.title,
            stake_credits: request.current_stake_credits,
            head_oid: &request.head_oid,
            assessment_outcome: request.assessment_outcome,
        })
    )
}

fn wait_label(ready_at_unix: Option<u64>, now_unix: u64) -> String {
    let Some(ready_at_unix) = ready_at_unix else {
        return "-".to_string();
    };
    let seconds = now_unix.saturating_sub(ready_at_unix);
    if seconds < 60 {
        "<1m".to_string()
    } else if seconds < 60 * 60 {
        format!("{}m", seconds / 60)
    } else if seconds < 24 * 60 * 60 {
        format!("{}h", seconds / (60 * 60))
    } else {
        format!("{}d", seconds / (24 * 60 * 60))
    }
}

fn request_detail_lines(request: &RequestSummaryResponse) -> Vec<String> {
    let mut lines = vec![
        request_line(request),
        format!(
            "  lifecycle: {} · {}",
            state_label(request.state, request.held_at_unix.is_some()),
            if request.first_ready_at_unix.is_some() {
                "published"
            } else {
                "not yet published"
            }
        ),
        format!(
            "  branch: {} · base {} {} · head {}",
            request.name,
            audience_label(request.audience),
            short_oid(&request.base_main_oid),
            short_oid(&request.head_oid)
        ),
        format!("  stake: {} credits", request.current_stake_credits),
    ];
    if !request.description_markdown.trim().is_empty() {
        lines.push(format!(
            "  description: {}",
            terminal_text(request.description_markdown.trim())
        ));
    }
    lines.push(match request.held_at_unix {
        Some(held_at) => format!("  hold: active since {held_at} · maintainer group"),
        None => "  hold: none".to_string(),
    });
    lines.push(if request.invitees.is_empty() {
        "  invitees: none".to_string()
    } else {
        format!(
            "  invitees: {}",
            request
                .invitees
                .iter()
                .map(|invitee| format!("@{}", terminal_text(&invitee.user.handle)))
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    lines.push(format!(
        "  capabilities: {}",
        capabilities_label(&request.permissions)
    ));
    lines.push(format!("  mergeability: {}", mergeability_label(request)));
    if let Some(outcome) = request.assessment_outcome {
        let mut assessment = format!("  assessment: {}", outcome_label(outcome));
        if let Some(body) = request.assessment_body_markdown.as_deref() {
            assessment.push_str(&format!(" · {}", terminal_text(body)));
        }
        if let Some(assessed_at) = request.assessed_at_unix {
            assessment.push_str(&format!(" · at {assessed_at}"));
        }
        lines.push(assessment);
    }
    if let Some(merged_at) = request.merged_at_unix {
        lines.push(format!(
            "  merge: {} → {} · at {merged_at}",
            request
                .merged_head_oid
                .as_deref()
                .map(short_oid)
                .unwrap_or_else(|| short_oid(&request.head_oid)),
            request
                .merged_main_oid
                .as_deref()
                .map(short_oid)
                .unwrap_or_else(|| "unknown".to_string())
        ));
    }
    lines
}

fn request_activity_lines(activity: &RequestActivityPageResponse) -> Vec<String> {
    let mut events = activity.events.iter().collect::<Vec<_>>();
    events.sort_by_key(|event| event.position);
    let mut cycle = 0_u64;
    let mut lines = Vec::new();
    for event in events {
        match &event.payload {
            RequestEventPayload::ReadyForReview {
                head_oid,
                stake_credits,
            } => {
                cycle += 1;
                let stake = if *stake_credits == 0 {
                    String::new()
                } else {
                    format!(" · {stake_credits} credits staked")
                };
                lines.push(format!(
                    "cycle {cycle}: Ready{stake} · head {} · at {}",
                    short_oid(head_oid),
                    event.created_at_unix
                ));
            }
            RequestEventPayload::ReturnedToWorking {
                stake_credits,
                reason,
                ..
            } => {
                let refund = if *stake_credits == 0 {
                    String::new()
                } else {
                    format!(" · {stake_credits} credits refunded")
                };
                lines.push(format!(
                    "cycle {}: Working{refund} · {} · at {}",
                    cycle.max(1),
                    exit_reason_label(*reason),
                    event.created_at_unix
                ));
            }
            RequestEventPayload::Settled { settlement } => lines.push(format!(
                "cycle {}: {} settlement · stake {} · refund {} · reward {} · burned {} · at {}",
                cycle.max(1),
                outcome_label(settlement.outcome),
                settlement.stake_credits,
                settlement.refunded_credits,
                settlement.reward_credits,
                settlement.burned_credits,
                settlement.settled_at_unix
            )),
            _ => {}
        }
    }
    lines
}

fn mutation_effect_lines(
    before: &RequestSummaryResponse,
    after: &RequestSummaryResponse,
) -> Vec<String> {
    let mut lines = Vec::new();
    match (before.state, after.state) {
        (RequestState::Working, RequestState::ReadyForReview)
            if after.current_stake_credits > 0 =>
        {
            lines.push(format!("Credits: {} staked", after.current_stake_credits));
        }
        (RequestState::ReadyForReview, RequestState::Working)
            if before.current_stake_credits > 0 =>
        {
            lines.push(format!(
                "Credits: {} refunded",
                before.current_stake_credits
            ));
        }
        _ => {}
    }
    if before.held_at_unix.is_some() && after.held_at_unix.is_none() {
        lines.push("Review hold cleared".to_string());
    } else if before.held_at_unix.is_none() && after.held_at_unix.is_some() {
        lines.push("Review hold started".to_string());
    }
    lines
}

struct RequestLine<'a> {
    name: &'a str,
    id: &'a str,
    state: RequestState,
    held: bool,
    title: &'a str,
    stake_credits: u32,
    head_oid: &'a str,
    assessment_outcome: Option<RequestAssessmentOutcome>,
}

fn format_request_line(line: RequestLine<'_>) -> String {
    let assessment = line
        .assessment_outcome
        .map(|outcome| format!(" · {}", outcome_label(outcome)))
        .unwrap_or_default();
    format!(
        "{:>2}  {:<9}  {} ({}) — {} · head {}{}",
        line.stake_credits,
        state_label(line.state, line.held),
        terminal_text(line.name),
        terminal_text(line.id),
        terminal_text(line.title),
        short_oid(line.head_oid),
        assessment
    )
}

fn capabilities_label(permissions: &RequestPermissionsResponse) -> String {
    let capabilities = [
        (permissions.can_push_branch, "push"),
        (permissions.can_pull_branch, "pull"),
        (permissions.can_mark_ready, "ready"),
        (permissions.can_return_to_working, "working"),
        (permissions.can_edit_identity, "edit"),
        (permissions.can_manage_invitees, "invitees"),
        (permissions.can_leave_request, "leave"),
        (permissions.can_hold, "hold"),
        (permissions.can_assess, "assess"),
        (permissions.can_merge, "merge"),
        (permissions.can_close, "close"),
        (permissions.can_open_discussion, "discuss"),
        (permissions.can_reply_to_discussion, "reply"),
    ]
    .into_iter()
    .filter_map(|(allowed, label)| allowed.then_some(label))
    .collect::<Vec<_>>();
    if capabilities.is_empty() {
        "view only".to_string()
    } else {
        capabilities.join(", ")
    }
}

fn mergeability_label(request: &RequestSummaryResponse) -> String {
    match request.mergeability.status {
        RequestMergeabilityStatus::Ready => "ready".to_string(),
        RequestMergeabilityStatus::Completed => "completed".to_string(),
        RequestMergeabilityStatus::Working => "working".to_string(),
        RequestMergeabilityStatus::NotMaintainer => request
            .mergeability
            .reason
            .clone()
            .unwrap_or_else(|| "repo maintainer required".to_string()),
        RequestMergeabilityStatus::MissingRequestBranch => request
            .mergeability
            .reason
            .clone()
            .unwrap_or_else(|| "request branch has not been pushed".to_string()),
    }
}

fn discussion_status_label(status: RequestDiscussionStatus) -> &'static str {
    match status {
        RequestDiscussionStatus::Dormant => "code-change",
        RequestDiscussionStatus::Open => "open",
        RequestDiscussionStatus::Resolved => "resolved",
    }
}

fn access_label(actor: RepositoryActor) -> &'static str {
    match actor {
        RepositoryActor::Owner => "owner",
        RepositoryActor::Member => "member",
        RepositoryActor::Public => "public contributor",
    }
}

fn audience_label(audience: RequestAudience) -> &'static str {
    match audience {
        RequestAudience::Public => "public main",
        RequestAudience::Private => "private main",
    }
}

fn state_label(state: RequestState, held: bool) -> &'static str {
    match (state, held) {
        (RequestState::ReadyForReview, true) => "on-hold",
        (RequestState::Working, _) => "working",
        (RequestState::ReadyForReview, false) => "ready",
        (RequestState::Completed, _) => "completed",
    }
}

fn outcome_label(outcome: RequestAssessmentOutcome) -> &'static str {
    match outcome {
        RequestAssessmentOutcome::Accepted => "Accepted",
        RequestAssessmentOutcome::Neutral => "Neutral",
        RequestAssessmentOutcome::Rejected => "Rejected",
    }
}

fn exit_reason_label(reason: RequestReviewExitReason) -> &'static str {
    match reason {
        RequestReviewExitReason::AuthorReturned => "author returned to Working",
        RequestReviewExitReason::ChangesRequested => "changes requested",
        RequestReviewExitReason::RevisionPushed => "revision pushed",
        RequestReviewExitReason::ContentEdited => "identity edited",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn list_uses_authoritative_response_hold_state() {
        let request: RequestListItemResponse = serde_json::from_value(json!({
            "id": "req_one", "name": "fix-refs", "title": "Fix refs",
            "author_role": "Public", "audience": "Public", "head_oid": oid('b'),
            "state": "ReadyForReview", "current_stake_credits": 12,
            "assessment_outcome": null, "ready_at_unix": 10,
            "held_at_unix": 11, "updated_at_unix": 20,
            "mergeability": {
                "status": "NotMaintainer",
                "current_main_oid": oid('a'),
                "request_head_oid": oid('b'),
                "reason": "repo maintainer required"
            }
        }))
        .unwrap();

        let rendered = request_list_line(&request, 70);
        assert!(rendered.contains("on-hold"), "{rendered}");
        assert!(rendered.contains("1m"), "{rendered}");
    }

    #[test]
    fn detail_uses_server_capabilities_and_renders_hold_invitees_and_publication() {
        let mut request = summary();
        request.state = RequestState::ReadyForReview;
        request.current_stake_credits = 12;
        request.first_ready_at_unix = Some(10);
        request.ready_at_unix = Some(11);
        request.held_at_unix = Some(12);
        request.held_by_user_id = Some("scope_usr_maintainer".to_string());
        request.permissions.can_edit_identity = true;
        request.permissions.can_hold = true;
        request.invitees = serde_json::from_value(json!([{
            "user": {"id": "scope_usr_devon", "handle": "devon"},
            "invited_by_user_id": "scope_usr_author",
            "created_at_unix": 5
        }]))
        .unwrap();

        let rendered = request_detail_lines(&request).join("\n");

        assert!(rendered.contains("on-hold"), "{rendered}");
        assert!(rendered.contains("published"), "{rendered}");
        assert!(rendered.contains("12 credits"), "{rendered}");
        assert!(rendered.contains("@devon"), "{rendered}");
        assert!(rendered.contains("edit, hold"), "{rendered}");
    }

    #[test]
    fn activity_renders_repeated_cycles_refunds_and_exact_settlement() {
        let activity: RequestActivityPageResponse = serde_json::from_value(json!({
            "events": [
                event(4, json!({"Settled": {"settlement": {
                    "outcome": "Accepted", "stake_credits": 20,
                    "refunded_credits": 20, "reward_credits": 20,
                    "burned_credits": 0, "settled_at_unix": 40
                }}})),
                event(1, json!({"ReadyForReview": {"head_oid": oid('b'), "stake_credits": 10}})),
                event(3, json!({"ReadyForReview": {"head_oid": oid('c'), "stake_credits": 20}})),
                event(2, json!({"ReturnedToWorking": {
                    "head_oid": oid('b'), "stake_credits": 10, "reason": "RevisionPushed"
                }}))
            ],
            "through_position": 4
        }))
        .unwrap();

        let rendered = request_activity_lines(&activity).join("\n");

        assert!(
            rendered.contains("cycle 1: Ready · 10 credits staked"),
            "{rendered}"
        );
        assert!(
            rendered.contains("cycle 1: Working · 10 credits refunded"),
            "{rendered}"
        );
        assert!(
            rendered.contains("cycle 2: Ready · 20 credits staked"),
            "{rendered}"
        );
        assert!(
            rendered.contains("refund 20 · reward 20 · burned 0"),
            "{rendered}"
        );
    }

    #[test]
    fn zero_credit_cycles_do_not_claim_ledger_effects() {
        let activity: RequestActivityPageResponse = serde_json::from_value(json!({
            "events": [
                event(1, json!({"ReadyForReview": {"head_oid": oid('b'), "stake_credits": 0}})),
                event(2, json!({"ReturnedToWorking": {
                    "head_oid": oid('b'), "stake_credits": 0, "reason": "RevisionPushed"
                }}))
            ],
            "through_position": 2
        }))
        .unwrap();

        let rendered = request_activity_lines(&activity).join("\n");
        assert!(!rendered.contains("credits"), "{rendered}");
        assert!(rendered.contains("cycle 1: Ready · head"), "{rendered}");
        assert!(
            rendered.contains("cycle 1: Working · revision pushed"),
            "{rendered}"
        );
    }

    #[test]
    fn settlement_receipt_uses_the_latest_authoritative_event() {
        let activity: RequestActivityPageResponse = serde_json::from_value(json!({
            "events": [
                event(4, json!({"Settled": {"settlement": {
                    "outcome": "Rejected", "stake_credits": 12,
                    "refunded_credits": 0, "reward_credits": 0,
                    "burned_credits": 12, "settled_at_unix": 40
                }}}))
            ],
            "through_position": 4
        }))
        .unwrap();

        assert_eq!(
            settlement_effect_line(&activity).as_deref(),
            Some("Credits: stake 12 · refund 0 · reward 0 · burned 12")
        );
    }

    #[test]
    fn wait_labels_are_concise_and_saturating() {
        assert_eq!(wait_label(None, 3_600), "-");
        assert_eq!(wait_label(Some(3_590), 3_600), "<1m");
        assert_eq!(wait_label(Some(0), 3_600), "1h");
        assert_eq!(wait_label(Some(4_000), 3_600), "<1m");
    }

    #[test]
    fn mutation_effects_report_refunds_and_hold_clear() {
        let mut before = summary();
        before.state = RequestState::ReadyForReview;
        before.current_stake_credits = 25;
        before.held_at_unix = Some(12);
        let after = summary();

        assert_eq!(
            mutation_effect_lines(&before, &after),
            vec!["Credits: 25 refunded", "Review hold cleared"]
        );
    }

    fn summary() -> RequestSummaryResponse {
        serde_json::from_str(
            r#"{
                "id":"req_one","name":"fix-refs","title":"Fix request refs",
                "description_markdown":"Atomic updates","author_user_id":"scope_usr_author",
                "author_role":"Public","audience":"Public",
                "base_main_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "head_oid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","state":"Working",
                "activity_version":1,"current_stake_credits":0,
                "first_ready_at_unix":null,"ready_at_unix":null,"held_at_unix":null,
                "held_by_user_id":null,"assessment_outcome":null,
                "assessment_body_markdown":null,"assessed_at_unix":null,
                "assessed_by_user_id":null,"completed_at_unix":null,
                "completed_by_user_id":null,"merged_at_unix":null,"merged_by_user_id":null,
                "merged_head_oid":null,"merged_main_oid":null,"created_at_unix":1,
                "updated_at_unix":2,"invitees":[],
                "assessment_previews":[],
                "permissions":{"can_open_discussion":false,"can_reply_to_discussion":false,
                    "can_edit_identity":false,"can_pull_branch":false,"can_push_branch":false,
                    "can_mark_ready":false,"can_return_to_working":false,
                    "can_manage_invitees":false,"can_leave_request":false,"can_hold":false,
                    "can_request_changes":false,"can_assess":false,"can_close":false,"can_merge":false},
                "mergeability":{"status":"Working",
                    "current_main_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "request_head_oid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","reason":null}
            }"#,
        )
        .unwrap()
    }

    fn event(position: u64, payload: serde_json::Value) -> serde_json::Value {
        json!({
            "id": format!("event_{position}"),
            "position": position,
            "actor": {"id": "scope_usr_actor", "handle": "actor"},
            "kind": match payload.as_object().unwrap().keys().next().unwrap().as_str() {
                "ReadyForReview" => "ReadyForReview",
                "ReturnedToWorking" => "ReturnedToWorking",
                "Settled" => "Settled",
                _ => unreachable!()
            },
            "payload": payload,
            "created_at_unix": position * 10
        })
    }

    fn oid(character: char) -> String {
        std::iter::repeat_n(character, 40).collect()
    }
}
