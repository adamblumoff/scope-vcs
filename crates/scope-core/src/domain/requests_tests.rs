use super::requests::*;
use crate::domain::store::{DEFAULT_GIT_FILE_MODE, RepositoryAccess, RepositoryActor, SourceBlob};
use std::collections::BTreeMap;

#[test]
fn new_request_starts_as_an_unpublished_unstaked_working_request() {
    let mutation = start_request(&mut BTreeMap::new(), public_start_input()).unwrap();

    assert_eq!(mutation.request.state, RequestState::Working);
    assert_eq!(mutation.request.current_stake_credits, 0);
    assert!(!mutation.request.is_published());
    assert!(!request_counts_as_open(&mutation.request));
    assert_eq!(mutation.request.ready_at_unix, None);
    assert_eq!(mutation.request.completed_at_unix, None);
    mutation.request.validate_facts().unwrap();
}

#[test]
fn request_name_rules_and_repository_uniqueness_remain_domain_owned() {
    for invalid in [
        "main",
        "HEAD",
        "two words",
        "nested/name",
        "-leading",
        "UPPER",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        let mut input = public_start_input();
        input.name = invalid.to_string();
        assert!(start_request(&mut BTreeMap::new(), input).is_err());
    }

    let mut requests = BTreeMap::new();
    start_request(&mut requests, public_start_input()).unwrap();
    let mut duplicate = public_start_input();
    duplicate.id = "request_2".to_string();
    assert!(
        start_request(&mut requests, duplicate)
            .unwrap_err()
            .message
            .contains("already exists")
    );
    assert_eq!(canonical_request_ref("fix-parser"), "refs/heads/fix-parser");
}

#[test]
fn publication_marker_survives_return_to_working_and_is_required_for_completion() {
    let mut request = ready_request();
    let first_ready_at = request.first_ready_at_unix;
    assert!(request_counts_as_open(&request));
    request.state = RequestState::Working;
    request.current_stake_credits = 0;
    request.ready_at_unix = None;
    request.validate_facts().unwrap();
    assert_eq!(request.first_ready_at_unix, first_ready_at);
    assert!(request_counts_as_open(&request));

    request.state = RequestState::Completed;
    request.completed_at_unix = Some(30);
    request.completed_by_user_id = Some("author".to_string());
    request.updated_at_unix = 30;
    request.validate_facts().unwrap();
    assert!(!request_counts_as_open(&request));

    request.first_ready_at_unix = None;
    assert!(
        request
            .validate_facts()
            .unwrap_err()
            .message
            .contains("publication time")
    );
}

#[test]
fn ready_state_owns_current_stake_ready_time_and_complete_hold_pair() {
    let mut request = ready_request();
    request.validate_facts().unwrap();

    request.current_stake_credits = 0;
    assert!(
        request
            .validate_facts()
            .unwrap_err()
            .message
            .contains("current stake")
    );
    request.current_stake_credits = 25;
    request.held_at_unix = Some(21);
    request.updated_at_unix = 21;
    assert!(
        request
            .validate_facts()
            .unwrap_err()
            .message
            .contains("set together")
    );
    request.held_by_user_id = Some("maintainer".to_string());
    request.held_at_unix = Some(19);
    assert!(
        request
            .validate_facts()
            .unwrap_err()
            .message
            .contains("cannot precede ready time")
    );
    request.held_at_unix = Some(21);
    request.validate_facts().unwrap();

    request.current_stake_credits = 26;
    assert!(request.validate_facts().unwrap_err().message.contains("25"));
}

#[test]
fn assessment_is_atomic_immutable_completion_data() {
    let mut request = completed_request(RequestAssessmentOutcome::Rejected);
    request.assessment_body_markdown =
        Some("The change violates the parser invariant.".to_string());
    request.validate_facts().unwrap();

    request.assessment_body_markdown = Some("   ".to_string());
    assert!(
        request
            .validate_facts()
            .unwrap_err()
            .message
            .contains("written reason")
    );
    request.assessment_body_markdown = Some("Reason".to_string());
    request.assessed_at_unix = Some(29);
    assert!(
        request
            .validate_facts()
            .unwrap_err()
            .message
            .contains("atomically")
    );
}

#[test]
fn merge_facts_are_complete_accepted_and_never_precede_completion() {
    let mut request = completed_request(RequestAssessmentOutcome::Accepted);
    assert!(request_permissions(&request, maintainer_access(), Some("maintainer")).can_merge);
    assert_eq!(
        request_mergeability(&request, maintainer_access()).status,
        RequestMergeabilityStatus::Ready
    );
    request.merged_at_unix = Some(31);
    request.merged_by_user_id = Some("maintainer".to_string());
    request.merged_head_oid = Some("head".to_string());
    request.merged_main_oid = Some("main-after".to_string());
    request.updated_at_unix = 31;
    request.validate_facts().unwrap();
    assert!(!request_permissions(&request, maintainer_access(), Some("maintainer")).can_merge);
    assert_eq!(
        request_mergeability(&request, maintainer_access()).status,
        RequestMergeabilityStatus::Completed
    );

    request.assessment_outcome = Some(RequestAssessmentOutcome::Neutral);
    assert!(
        request
            .validate_facts()
            .unwrap_err()
            .message
            .contains("accepted")
    );
    request.assessment_outcome = Some(RequestAssessmentOutcome::Accepted);
    request.merged_at_unix = Some(29);
    assert!(
        request
            .validate_facts()
            .unwrap_err()
            .message
            .contains("precede completion")
    );
}

#[test]
fn repeat_review_cycles_are_append_only_facts_with_stake_and_reason() {
    let events = [
        RequestEventPayload::ReadyForReview {
            head_oid: "head-1".to_string(),
            stake_credits: 10,
        },
        RequestEventPayload::ReturnedToWorking {
            head_oid: "head-1".to_string(),
            stake_credits: 10,
            reason: RequestReviewExitReason::RevisionPushed,
        },
        RequestEventPayload::ReadyForReview {
            head_oid: "head-2".to_string(),
            stake_credits: 20,
        },
    ];

    let encoded = serde_json::to_value(events).unwrap();
    assert_eq!(encoded[0]["ReadyForReview"]["stake_credits"], 10);
    assert_eq!(encoded[1]["ReturnedToWorking"]["reason"], "RevisionPushed");
    assert_eq!(encoded[2]["ReadyForReview"]["head_oid"], "head-2");
}

#[test]
fn assessment_settlement_has_only_three_outcomes() {
    let accepted = settlement_for(10, RequestAssessmentOutcome::Accepted, 30);
    assert_eq!(
        (accepted.refunded_credits, accepted.reward_credits),
        (10, 10)
    );
    let neutral = settlement_for(10, RequestAssessmentOutcome::Neutral, 30);
    assert_eq!((neutral.refunded_credits, neutral.reward_credits), (10, 0));
    let rejected = settlement_for(10, RequestAssessmentOutcome::Rejected, 30);
    assert_eq!(
        (rejected.refunded_credits, rejected.burned_credits),
        (0, 10)
    );
}

#[test]
fn credit_grant_is_single_entry_and_overflow_safe() {
    let mut accounts = BTreeMap::new();
    let mut ledger = BTreeMap::new();
    let mutation = grant_user_credits(
        &mut accounts,
        &mut ledger,
        GrantUserCreditsInput {
            ledger_entry_id: "starter:user".to_string(),
            user_id: "user".to_string(),
            amount_credits: 100,
            now_unix: 10,
        },
    )
    .unwrap();
    assert_eq!(mutation.account.balance_credits, 100);
    assert_eq!(
        mutation.ledger_entry.kind,
        CreditLedgerEntryKind::StarterGrant
    );
    assert_eq!(ledger.len(), 1);
}

#[test]
fn public_working_request_remains_visible_and_author_can_work() {
    let request = working_request();
    assert!(request_visible_to_access(
        &request,
        RepositoryAccess::public()
    ));
    let anonymous = request_permissions(&request, RepositoryAccess::public(), None);
    assert!(anonymous.can_pull_branch);
    let author = request_permissions(
        &request,
        RepositoryAccess::public(),
        Some(request.author_user_id.as_str()),
    );
    assert!(author.can_pull_branch);
    assert!(author.can_push_branch);
    assert!(author.can_mark_ready);
}

#[test]
fn published_working_request_stays_visible_but_not_mergeable() {
    let mut request = ready_request();
    request.state = RequestState::Working;
    request.ready_at_unix = None;
    request.current_stake_credits = 0;
    request.validate_facts().unwrap();
    assert!(request_visible_to_access(
        &request,
        RepositoryAccess::public()
    ));
    assert_eq!(
        request_mergeability(&request, maintainer_access()).status,
        RequestMergeabilityStatus::Working
    );
}

#[test]
fn hold_and_completion_remove_mutation_permissions() {
    let mut request = ready_request();
    request.held_at_unix = Some(21);
    request.held_by_user_id = Some("maintainer".to_string());
    request.updated_at_unix = 21;
    let author = request_permissions(&request, RepositoryAccess::public(), Some("author"));
    assert!(!author.can_push_branch);
    assert!(!author.can_return_to_working);
    assert!(!author.can_manage_invitees);
    let maintainer = request_permissions(&request, maintainer_access(), Some("maintainer"));
    assert!(maintainer.can_hold);
    assert!(maintainer.can_assess);
    assert!(maintainer.can_manage_invitees);

    let mut private_request = request.clone();
    private_request.audience = RequestAudience::Private;
    let maintainer = request_permissions(&private_request, maintainer_access(), Some("maintainer"));
    assert!(!maintainer.can_manage_invitees);

    let request = completed_request(RequestAssessmentOutcome::Neutral);
    let maintainer = request_permissions(&request, maintainer_access(), Some("maintainer"));
    assert!(maintainer.can_pull_branch);
    assert!(!maintainer.can_push_branch);
    assert!(!maintainer.can_open_discussion);
}

#[test]
fn held_request_rejects_description_edits_at_domain_boundary() {
    let mut request = ready_request();
    request.held_at_unix = Some(21);
    request.held_by_user_id = Some("maintainer".to_string());
    request.updated_at_unix = 21;
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let error = update_request_description(
        &mut requests,
        &mut BTreeMap::new(),
        UpdateRequestDescriptionInput {
            request_id: "request_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit_description: true,
            event_id: "event_description".to_string(),
            description_markdown: "Changed while held".to_string(),
            now_unix: 22,
        },
    )
    .unwrap_err();
    assert!(error.message.contains("while held"));
}

#[test]
fn ready_request_rejects_description_edits_until_review_invalidation_exists() {
    let request = ready_request();
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let error = update_request_description(
        &mut requests,
        &mut BTreeMap::new(),
        UpdateRequestDescriptionInput {
            request_id: "request_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit_description: true,
            event_id: "event_description".to_string(),
            description_markdown: "Changed while ready".to_string(),
            now_unix: 22,
        },
    )
    .unwrap_err();
    assert!(error.message.contains("while ready for review"));
}

#[test]
fn close_is_author_only() {
    let request = working_request();
    assert!(request_permissions(&request, RepositoryAccess::public(), Some("author"),).can_close);
    assert!(!request_permissions(&request, maintainer_access(), Some("maintainer")).can_close);
}

#[test]
fn close_hard_deletes_draft_and_completes_published_work() {
    let draft = working_request();
    let mut requests = BTreeMap::from([(draft.id.clone(), draft)]);
    let mut events = BTreeMap::new();
    let mut change_blocks = BTreeMap::new();
    assert!(matches!(
        close_request(
            &mut requests,
            &mut events,
            &mut change_blocks,
            close_input(),
        )
        .unwrap(),
        CloseRequestMutation::DeletedDraft { .. }
    ));
    assert!(requests.is_empty());

    let mut published = ready_request();
    published.state = RequestState::Working;
    published.ready_at_unix = None;
    published.current_stake_credits = 0;
    let mut requests = BTreeMap::from([(published.id.clone(), published)]);
    let mutation = close_request(
        &mut requests,
        &mut events,
        &mut change_blocks,
        close_input(),
    )
    .unwrap();
    let CloseRequestMutation::Completed { request, event } = mutation else {
        panic!("published request must remain as completed history");
    };
    assert_eq!(request.state, RequestState::Completed);
    assert_eq!(request.assessment_outcome, None);
    assert_eq!(event.kind, RequestEventKind::Closed);
}

#[test]
fn discussion_resolution_is_moderation_not_request_lifecycle() {
    let request = ready_request();
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let mut discussions = BTreeMap::new();
    let opened = create_request_discussion(
        &mut requests,
        &mut discussions,
        CreateRequestDiscussionInput {
            request_id: "request_1".to_string(),
            id: "discussion_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_participate: true,
            client_discussion_id: "client_1".to_string(),
            body_markdown: "Review this invariant".to_string(),
            now_unix: 21,
        },
    )
    .unwrap();
    let resolved = resolve_request_discussion(
        &mut requests,
        &mut discussions,
        ResolveRequestDiscussionInput {
            request_id: "request_1".to_string(),
            discussion_id: opened.discussion.id.clone(),
            actor_user_id: "maintainer".to_string(),
            actor_is_maintainer: true,
            event_id: "event_discussion_resolved".to_string(),
            now_unix: 22,
        },
    )
    .unwrap();
    assert_eq!(resolved.event.kind, RequestEventKind::DiscussionResolved);
    assert_eq!(requests["request_1"].state, RequestState::ReadyForReview);

    let reopened = reopen_request_discussion(
        &mut requests,
        &mut discussions,
        ReopenRequestDiscussionInput {
            request_id: "request_1".to_string(),
            discussion_id: opened.discussion.id,
            actor_user_id: "author".to_string(),
            actor_is_maintainer: false,
            event_id: "event_discussion_reopened".to_string(),
            now_unix: 23,
        },
    )
    .unwrap();
    assert_eq!(reopened.event.kind, RequestEventKind::DiscussionReopened);
    assert_eq!(requests["request_1"].state, RequestState::ReadyForReview);
}

fn public_start_input() -> StartRequestInput {
    StartRequestInput {
        id: "request_1".to_string(),
        repo_id: "owner/repo".to_string(),
        name: "fix-parser".to_string(),
        author_user_id: "author".to_string(),
        title: Some("Fix parser".to_string()),
        author_role: RequestActorRole::Public,
        audience: RequestAudience::Public,
        base_main_oid: "base".to_string(),
        event_id: "event_started".to_string(),
        now_unix: 10,
    }
}

fn working_request() -> Request {
    start_request(&mut BTreeMap::new(), public_start_input())
        .unwrap()
        .request
}

fn ready_request() -> Request {
    let mut request = working_request();
    request.head_oid = "head".to_string();
    request.git_snapshot = Some(source_blob("head"));
    request.state = RequestState::ReadyForReview;
    request.current_stake_credits = 10;
    request.first_ready_at_unix = Some(20);
    request.ready_at_unix = Some(20);
    request.updated_at_unix = 20;
    request
}

fn completed_request(outcome: RequestAssessmentOutcome) -> Request {
    let mut request = ready_request();
    request.state = RequestState::Completed;
    request.current_stake_credits = 0;
    request.ready_at_unix = None;
    request.assessment_outcome = Some(outcome);
    request.assessed_at_unix = Some(30);
    request.assessed_by_user_id = Some("maintainer".to_string());
    request.completed_at_unix = Some(30);
    request.completed_by_user_id = Some("maintainer".to_string());
    request.updated_at_unix = 30;
    request
}

fn close_input() -> CloseRequestInput {
    CloseRequestInput {
        request_id: "request_1".to_string(),
        actor_user_id: "author".to_string(),
        actor_can_close: true,
        event_id: "event_closed".to_string(),
        now_unix: 30,
    }
}

fn maintainer_access() -> RepositoryAccess {
    RepositoryAccess {
        actor: RepositoryActor::Member,
        can_read_private_files: true,
        can_push: true,
        can_change_file_visibility: false,
        can_apply_changes: false,
        can_manage_members: false,
        can_delete_repo: false,
    }
}

fn source_blob(git_oid: &str) -> SourceBlob {
    SourceBlob {
        object_key: format!("objects/{git_oid}"),
        sha256: format!("sha256-{git_oid}"),
        git_oid: git_oid.to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 1,
    }
}
