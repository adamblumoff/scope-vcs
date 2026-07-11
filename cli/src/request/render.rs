use super::text::{short_oid, terminal_text};
use crate::api::{
    RepoSummaryResponse, RepositoryActor, RequestDetailResponse, RequestEventResponse,
    RequestMergeabilityStatus, RequestMutationResponse, RequestSummaryResponse,
};
use anyhow::{Context, bail};
use scope_core::domain::requests::{RequestAudience, RequestEventKind, RequestState};
use std::io::{self, Write};

pub(super) fn print_repo_access(repo: &RepoSummaryResponse) {
    println!(
        "Scope repo: {}/{}",
        repo.owner_handle.as_str(),
        repo.name.as_str()
    );
    println!("Permission: {}", access_label(repo.access.actor));
    if repo.request_permissions.uses_credit_stake {
        println!("Credit stake: required on first submit");
    } else {
        println!("Credit stake: not used for owner/member requests");
    }
}

pub(super) fn print_submit_stake(stake_credits: Option<u32>) {
    match stake_credits {
        Some(stake) => println!("Stake: {stake} credits on first submit"),
        None => println!("Stake: not used for this request"),
    }
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
    if let Some(settlement) = &detail.request.settlement {
        println!(
            "  settlement: refunded={} reward={} burned={}",
            settlement.refunded_credits, settlement.reward_credits, settlement.burned_credits
        );
    }
    if !detail.events.is_empty() {
        println!("  events:");
        for event in &detail.events {
            println!("    {}", event_line(event));
        }
    }
}

pub(super) fn print_mutation_receipt(action: &str, response: &RequestMutationResponse) {
    println!("{action}: {}", request_line(&response.request));
    if let Some(settlement) = &response.request.settlement {
        println!(
            "Settlement: refunded={} reward={} burned={}",
            settlement.refunded_credits, settlement.reward_credits, settlement.burned_credits
        );
    }
}

pub(super) fn ensure_mergeable(request: &RequestSummaryResponse) -> anyhow::Result<()> {
    if !request.permissions.can_merge {
        bail!("you do not have permission to merge request {}", request.id);
    }
    if request.mergeability.status == RequestMergeabilityStatus::Ready {
        return Ok(());
    }
    let reason = request
        .mergeability
        .reason
        .as_deref()
        .unwrap_or("request is not cleanly mergeable");
    bail!("request {} cannot be merged: {reason}", request.id)
}

pub(super) fn confirm_merge(request: &RequestSummaryResponse) -> anyhow::Result<()> {
    let current_main_oid = request
        .mergeability
        .current_main_oid
        .as_deref()
        .context("request has no current main oid to merge into")?;
    println!(
        "Are you sure you want to merge request {} into main at {}?",
        request.id,
        short_oid(current_main_oid)
    );
    print!("Type 'merge' to continue: ");
    io::stdout().flush().context("flush merge confirmation")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("read merge confirmation")?;
    if input.trim() != "merge" {
        bail!("merge cancelled");
    }
    Ok(())
}

pub(super) fn request_line(request: &RequestSummaryResponse) -> String {
    let settlement = request
        .settlement
        .as_ref()
        .map(|settlement| {
            format!(
                " settlement(refund={} reward={} burn={})",
                settlement.refunded_credits, settlement.reward_credits, settlement.burned_credits
            )
        })
        .unwrap_or_default();
    format!(
        "{} ({}) [{}] {} stake={} head={}{}",
        request.name,
        request.id,
        state_label(request.state),
        terminal_text(&request.title),
        request.stake_credits,
        short_oid(&request.head_oid),
        settlement
    )
}

fn event_line(event: &RequestEventResponse) -> String {
    let body = event
        .body
        .as_deref()
        .map(|body| format!(": {}", terminal_text(body)))
        .unwrap_or_default();
    let head = event
        .new_head_oid
        .as_deref()
        .map(|oid| format!(" head={}", short_oid(oid)))
        .unwrap_or_default();
    format!("{}{}{}", event_kind_label(event.kind), head, body)
}

fn mergeability_label(request: &RequestSummaryResponse) -> String {
    match request.mergeability.status {
        RequestMergeabilityStatus::Ready => "ready".to_string(),
        RequestMergeabilityStatus::Closed => request
            .mergeability
            .reason
            .clone()
            .unwrap_or_else(|| "closed".to_string()),
        RequestMergeabilityStatus::NotReady => request
            .mergeability
            .reason
            .clone()
            .unwrap_or_else(|| "request is not ready to merge".to_string()),
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
        RequestState::Submitted => "submitted",
        RequestState::NeedsResponse => "needs-response",
        RequestState::Resolved => "resolved",
        RequestState::Withdrawn => "withdrawn",
    }
}

fn event_kind_label(kind: RequestEventKind) -> &'static str {
    match kind {
        RequestEventKind::Started => "started",
        RequestEventKind::Submitted => "submitted",
        RequestEventKind::RevisionPushed => "revision-pushed",
        RequestEventKind::Commented => "commented",
        RequestEventKind::NeedsResponse => "needs-response",
        RequestEventKind::ContributorResponded => "contributor-responded",
        RequestEventKind::Merged => "merged",
        RequestEventKind::Resolved => "resolved",
        RequestEventKind::Settled => "settled",
        RequestEventKind::Withdrawn => "withdrawn",
    }
}
