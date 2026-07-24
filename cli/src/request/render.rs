use super::text::{short_oid, terminal_text};
use crate::api::{
    RepoSummaryResponse, RepositoryActor, RequestDetailResponse, RequestDiscussionMutationResponse,
    RequestListItemResponse, RequestMergeabilityStatus, RequestSummaryResponse,
};
use scope_core::domain::requests::{RequestAudience, RequestDiscussionStatus, RequestState};

pub(super) fn print_repo_access(repo: &RepoSummaryResponse) {
    println!(
        "Scope repo: {}/{}",
        repo.owner_handle.as_str(),
        repo.name.as_str()
    );
    println!("Permission: {}", access_label(repo.access.actor));
    println!(
        "Credit stake: {}",
        if repo.request_permissions.uses_credit_stake {
            "used when entering review"
        } else {
            "not used for owner/member requests"
        }
    );
}

pub(super) fn print_request_detail(detail: &RequestDetailResponse) {
    println!("{}", request_line(&detail.request));
    println!(
        "  base: {} {}",
        audience_label(detail.request.audience),
        short_oid(&detail.request.base_main_oid)
    );
    println!(
        "  head: {} at origin/{}",
        short_oid(&detail.request.head_oid),
        detail.request.name
    );
    println!("  mergeability: {}", mergeability_label(&detail.request));
    if let Some(outcome) = detail.request.assessment_outcome {
        println!("  assessment: {outcome:?}");
    }
    if let Some(merged_at) = detail.request.merged_at_unix {
        println!("  merged at: {merged_at}");
    }
}

pub(super) fn print_discussion_receipt(response: &RequestDiscussionMutationResponse) {
    let discussion = &response.discussion;
    println!(
        "Discussion opened: {} [{}] by @{}",
        discussion.id,
        discussion_status_label(discussion.status),
        discussion.author.handle
    );
    println!("Replies: {}", discussion.reply_count);
    println!(
        "{}",
        terminal_text(discussion.body_markdown.as_deref().unwrap_or("Code change"))
    );
}

fn discussion_status_label(status: RequestDiscussionStatus) -> &'static str {
    match status {
        RequestDiscussionStatus::Dormant => "code-change",
        RequestDiscussionStatus::Open => "open",
        RequestDiscussionStatus::Resolved => "resolved",
    }
}

pub(super) fn request_line(request: &RequestSummaryResponse) -> String {
    format_request_line(
        &request.name,
        &request.id,
        request.state,
        &request.title,
        request.current_stake_credits,
        &request.head_oid,
        request.assessment_outcome,
    )
}

pub(super) fn request_list_line(request: &RequestListItemResponse) -> String {
    format_request_line(
        &request.name,
        &request.id,
        request.state,
        &request.title,
        request.current_stake_credits,
        &request.head_oid,
        request.assessment_outcome,
    )
}

fn format_request_line(
    name: &str,
    id: &str,
    state: RequestState,
    title: &str,
    stake_credits: u32,
    head_oid: &str,
    assessment_outcome: Option<crate::api::RequestAssessmentOutcome>,
) -> String {
    let assessment = assessment_outcome
        .map(|outcome| format!(" assessment={outcome:?}"))
        .unwrap_or_default();
    format!(
        "{} ({}) [{}] {} stake={} head={}{}",
        name,
        id,
        state_label(state),
        terminal_text(title),
        stake_credits,
        short_oid(head_oid),
        assessment
    )
}

fn mergeability_label(request: &RequestSummaryResponse) -> String {
    match request.mergeability.status {
        RequestMergeabilityStatus::Ready => "ready".to_string(),
        RequestMergeabilityStatus::Completed => "completed".to_string(),
        RequestMergeabilityStatus::Working => "working".to_string(),
        RequestMergeabilityStatus::Held => "on hold".to_string(),
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
        RequestState::Working => "working",
        RequestState::ReadyForReview => "ready-for-review",
        RequestState::Completed => "completed",
    }
}
