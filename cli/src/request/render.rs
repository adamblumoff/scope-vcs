use super::text::{short_oid, terminal_text};
use crate::api::{
    LeaveRequestResponse, RepoSummaryResponse, RepositoryActor, RequestActivityPageResponse,
    RequestAudience, RequestCloseResponse, RequestDetailResponse,
    RequestDiscussionMutationResponse, RequestDiscussionStatus, RequestEventPayload,
    RequestInviteeMutationResponse, RequestListItemResponse, RequestMergeabilityStatus,
    RequestMutationResponse, RequestPermissionsResponse, RequestState, RequestSummaryResponse,
};

pub(super) fn repo_access_lines(repo: &RepoSummaryResponse) -> Vec<String> {
    vec![
        format!("Scope repo: {}/{}", repo.owner_handle, repo.name),
        format!("Permission: {}", access_label(repo.access.actor)),
    ]
}

pub(super) fn request_detail_lines_for_response(detail: &RequestDetailResponse) -> Vec<String> {
    request_detail_lines(&detail.request)
}

pub(super) fn request_activity_lines_for_response(
    activity: &RequestActivityPageResponse,
) -> Vec<String> {
    let lines = request_activity_lines(activity);
    if lines.is_empty() {
        return Vec::new();
    }
    let mut rendered = vec!["Activity:".to_string()];
    for line in lines {
        rendered.push(format!("  {line}"));
    }
    rendered
}

pub(super) fn request_mutation_receipt_lines(
    action: &str,
    before: Option<&RequestSummaryResponse>,
    response: &RequestMutationResponse,
) -> Vec<String> {
    let action = terminal_text(action);
    let mut lines = vec![format!("{action} · {}", request_line(&response.request))];
    if let Some(before) = before {
        lines.extend(mutation_effect_lines(before, &response.request));
    }
    lines
}

pub(super) fn invitee_added_receipt(response: &RequestInviteeMutationResponse) -> String {
    format!(
        "Invited @{} · can now push request {}",
        terminal_text(&response.invitee.user.handle),
        response.request.name
    )
}

pub(super) fn invitee_removed_receipt(response: &RequestInviteeMutationResponse) -> String {
    format!(
        "Removed @{} from request {}",
        terminal_text(&response.invitee.user.handle),
        response.request.name
    )
}

pub(super) fn leave_receipt(request_id: &str, response: &LeaveRequestResponse) -> String {
    format!(
        "@{} left request {}",
        terminal_text(&response.invitee.user.handle),
        request_id
    )
}

pub(super) fn close_receipt(request_id: &str, response: &RequestCloseResponse) -> String {
    if response.deleted {
        format!("Closed and removed draft request {request_id}")
    } else if let Some(request) = response.request.as_ref() {
        format!("Closed request {} · remains in history", request.name)
    } else {
        format!("Closed request {request_id}")
    }
}

pub(super) fn discussion_receipt_lines(
    response: &RequestDiscussionMutationResponse,
) -> Vec<String> {
    let discussion = &response.discussion;
    vec![
        format!(
            "Discussion opened: {} [{}] by @{}",
            discussion.id,
            discussion_status_label(discussion.status),
            terminal_text(&discussion.author.handle)
        ),
        format!("Replies: {}", discussion.reply_count),
        terminal_text(&discussion.body_markdown),
    ]
}

pub(super) fn request_line(request: &RequestSummaryResponse) -> String {
    format_request_line(RequestLine {
        name: &request.name,
        id: &request.id,
        state: request.state,
        title: &request.title,
        head_oid: &request.head_oid,
    })
}

pub(super) fn request_list_line(request: &RequestListItemResponse, now_unix: u64) -> String {
    format!(
        "{:>5}  {}",
        wait_label(request.submitted_at_unix, now_unix),
        format_request_line(RequestLine {
            name: &request.name,
            id: &request.id,
            state: request.state,
            title: &request.title,
            head_oid: &request.head_oid,
        })
    )
}

fn wait_label(submitted_at_unix: Option<u64>, now_unix: u64) -> String {
    let Some(submitted_at_unix) = submitted_at_unix else {
        return "-".to_string();
    };
    let seconds = now_unix.saturating_sub(submitted_at_unix);
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
            state_label(request.state),
            if request.submitted_at_unix.is_some() {
                "submitted"
            } else {
                "not yet submitted"
            }
        ),
        format!(
            "  branch: {} · base {} {} · head {}",
            request.name,
            audience_label(request.audience),
            short_oid(&request.base_main_oid),
            short_oid(&request.head_oid)
        ),
    ];
    if !request.description_markdown.trim().is_empty() {
        lines.push(format!(
            "  description: {}",
            terminal_text(request.description_markdown.trim())
        ));
    }
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
    let mut lines = Vec::new();
    for event in events {
        if let RequestEventPayload::Submitted { head_oid } = &event.payload {
            lines.push(format!(
                "Submitted · head {} · at {}",
                short_oid(head_oid),
                event.created_at_unix
            ));
        }
    }
    lines
}

fn mutation_effect_lines(
    _before: &RequestSummaryResponse,
    _after: &RequestSummaryResponse,
) -> Vec<String> {
    Vec::new()
}

struct RequestLine<'a> {
    name: &'a str,
    id: &'a str,
    state: RequestState,
    title: &'a str,
    head_oid: &'a str,
}

fn format_request_line(line: RequestLine<'_>) -> String {
    format!(
        "{:<9}  {} ({}) — {} · head {}",
        state_label(line.state),
        terminal_text(line.name),
        terminal_text(line.id),
        terminal_text(line.title),
        short_oid(line.head_oid)
    )
}

fn capabilities_label(permissions: &RequestPermissionsResponse) -> String {
    let capabilities = [
        (permissions.can_push_branch, "push"),
        (permissions.can_pull_branch, "pull"),
        (permissions.can_submit, "submit"),
        (permissions.can_edit_identity, "edit"),
        (permissions.can_manage_invitees, "invitees"),
        (permissions.can_leave_request, "leave"),
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
        RequestMergeabilityStatus::Closed => "closed".to_string(),
        RequestMergeabilityStatus::Merged => "merged".to_string(),
        RequestMergeabilityStatus::Draft => "draft".to_string(),
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

fn state_label(state: RequestState) -> &'static str {
    match state {
        RequestState::Draft => "draft",
        RequestState::Open => "open",
        RequestState::Closed => "closed",
        RequestState::Merged => "merged",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn list_renders_open_state_and_wait() {
        let request: RequestListItemResponse = serde_json::from_value(json!({
            "id": "req_one", "name": "fix-refs", "title": "Fix refs",
            "author_role": "Public", "audience": "Public", "head_oid": oid('b'),
            "state": "Open", "submitted_at_unix": 10, "updated_at_unix": 20,
            "mergeability": {
                "status": "NotMaintainer",
                "current_main_oid": oid('a'),
                "request_head_oid": oid('b'),
                "reason": "repo maintainer required"
            }
        }))
        .unwrap();

        let rendered = request_list_line(&request, 70);
        assert!(rendered.contains("open"), "{rendered}");
        assert!(rendered.contains("1m"), "{rendered}");
    }

    #[test]
    fn detail_uses_server_capabilities_and_renders_invitees_and_submission() {
        let mut request = summary();
        request.state = RequestState::Open;
        request.submitted_at_unix = Some(10);
        request.permissions.can_edit_identity = true;
        request.invitees = serde_json::from_value(json!([{
            "user": {"id": "scope_usr_devon", "handle": "devon"},
            "invited_by_user_id": "scope_usr_author",
            "created_at_unix": 5
        }]))
        .unwrap();

        let rendered = request_detail_lines(&request).join("\n");

        assert!(rendered.contains("open"), "{rendered}");
        assert!(rendered.contains("submitted"), "{rendered}");
        assert!(rendered.contains("@devon"), "{rendered}");
        assert!(rendered.contains("edit"), "{rendered}");
    }

    #[test]
    fn activity_renders_submission() {
        let activity: RequestActivityPageResponse = serde_json::from_value(json!({
            "events": [
                event(1, json!({"Submitted": {"head_oid": oid('b')}}))
            ],
            "through_position": 1
        }))
        .unwrap();

        let rendered = request_activity_lines(&activity).join("\n");

        assert!(rendered.contains("Submitted · head"), "{rendered}");
    }

    #[test]
    fn wait_labels_are_concise_and_saturating() {
        assert_eq!(wait_label(None, 3_600), "-");
        assert_eq!(wait_label(Some(3_590), 3_600), "<1m");
        assert_eq!(wait_label(Some(0), 3_600), "1h");
        assert_eq!(wait_label(Some(4_000), 3_600), "<1m");
    }

    fn summary() -> RequestSummaryResponse {
        serde_json::from_str(
            r#"{
                "id":"req_one","name":"fix-refs","title":"Fix request refs",
                "description_markdown":"Atomic updates","author_user_id":"scope_usr_author",
                "author_role":"Public","audience":"Public",
                "base_main_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "head_oid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","state":"Draft",
                "activity_version":1,
                "submitted_at_unix":null,"closed_at_unix":null,"closed_by_user_id":null,
                "merged_at_unix":null,"merged_by_user_id":null,
                "merged_head_oid":null,"merged_main_oid":null,"created_at_unix":1,
                "updated_at_unix":2,"invitees":[],
                "permissions":{"can_view_activity":false,"can_open_discussion":false,"can_reply_to_discussion":false,
                    "can_edit_identity":false,"can_pull_branch":false,"can_push_branch":false,
                    "can_submit":false,
                    "can_manage_invitees":false,"can_leave_request":false,
                    "can_close":false,"can_merge":false},
                "mergeability":{"status":"Draft",
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
                "Submitted" => "Submitted",
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
