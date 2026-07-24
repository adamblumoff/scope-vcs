use super::requests::*;
use crate::domain::store::{DEFAULT_GIT_FILE_MODE, RepositoryAccess, RepositoryActor, SourceBlob};
use std::collections::BTreeMap;

#[test]
fn new_request_starts_as_an_unpublished_unstaked_working_request() {
    let mutation = start_request(&mut BTreeMap::new(), public_start_input()).unwrap();

    assert_eq!(mutation.request.state, RequestState::Working);
    assert_eq!(mutation.request.current_stake_credits, 0);
    assert!(!mutation.request.is_published());
    assert!(!policy_for(&mutation.request, ViewerKind::Anonymous).counts_as_ready);
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
    assert!(policy_for(&request, ViewerKind::Anonymous).counts_as_ready);
    request.state = RequestState::Working;
    request.current_stake_credits = 0;
    request.ready_at_unix = None;
    request.validate_facts().unwrap();
    assert_eq!(request.first_ready_at_unix, first_ready_at);
    assert!(!policy_for(&request, ViewerKind::Anonymous).counts_as_ready);

    request.state = RequestState::Completed;
    request.completed_at_unix = Some(30);
    request.completed_by_user_id = Some("author".to_string());
    request.updated_at_unix = 30;
    request.validate_facts().unwrap();
    assert!(!policy_for(&request, ViewerKind::Anonymous).counts_as_ready);

    request.first_ready_at_unix = None;
    request.ready_queue_version = None;
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
    assert!(
        policy_for(&request, ViewerKind::Maintainer)
            .permissions
            .can_merge
    );
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
    assert!(
        !policy_for(&request, ViewerKind::Maintainer)
            .permissions
            .can_merge
    );
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
fn request_policy_surface_truth_table_is_viewer_and_lifecycle_aware() {
    let never_published_working = working_request();
    let published_working = published_working_request();
    let ready = ready_request();
    let held_ready = held_request();
    let completed = completed_request(RequestAssessmentOutcome::Neutral);
    let private_working = private_request(working_request());
    let private_published_working = private_request(published_working_request());
    let private_ready = private_request(ready_request());
    let private_completed = private_request(completed_request(RequestAssessmentOutcome::Neutral));

    let cases = [
        (
            "never-published public Working",
            &never_published_working,
            [
                hidden(),
                hidden(),
                collaborator_working(),
                collaborator_working(),
                exact_only_working(),
            ],
        ),
        (
            "previously-published public Working",
            &published_working,
            [
                published_working_reader(false),
                published_working_reader(false),
                collaborator_working(),
                collaborator_working(),
                published_working_reader(true),
            ],
        ),
        (
            "public Ready",
            &ready,
            [
                published_ready_reader(false),
                published_ready_reader(false),
                published_ready_reader(true),
                published_ready_reader(true),
                published_ready_reader(true),
            ],
        ),
        (
            "held public Ready",
            &held_ready,
            [
                published_ready_reader(false),
                published_ready_reader(false),
                published_ready_reader(false),
                published_ready_reader(false),
                published_ready_reader(true),
            ],
        ),
        ("public completed", &completed, [published_history(); 5]),
        (
            "private Working",
            &private_working,
            [
                hidden(),
                hidden(),
                private_visible(false, true),
                hidden(),
                private_visible(false, true),
            ],
        ),
        (
            "previously-published private Working",
            &private_published_working,
            [
                hidden(),
                hidden(),
                private_visible(false, true),
                hidden(),
                private_visible(false, true),
            ],
        ),
        (
            "private Ready",
            &private_ready,
            [
                hidden(),
                hidden(),
                private_visible(true, true),
                hidden(),
                private_visible(true, true),
            ],
        ),
        (
            "private completed",
            &private_completed,
            [
                hidden(),
                hidden(),
                private_visible(false, false),
                hidden(),
                private_visible(false, false),
            ],
        ),
    ];

    for (request_label, request, expected_by_viewer) in cases {
        for (viewer, expected) in ViewerKind::ALL.into_iter().zip(expected_by_viewer) {
            let actual = policy_for(request, viewer);
            assert_surface_decision(request_label, viewer, actual, expected);
            if !actual.exact_visible {
                assert_eq!(
                    actual.permissions,
                    no_permissions(),
                    "{request_label} / {viewer:?} must expose no capabilities"
                );
            }
        }
    }

    assert_eq!(
        request_mergeability(&published_working, maintainer_access()).status,
        RequestMergeabilityStatus::Working
    );
}

#[test]
fn request_policy_permissions_keep_roles_and_hold_behavior_distinct() {
    let working = working_request();
    let author = policy_for(&working, ViewerKind::Author).permissions;
    assert!(author.can_open_discussion && author.can_reply_to_discussion);
    assert!(author.can_edit_description && author.can_pull_branch && author.can_push_branch);
    assert!(author.can_mark_ready && author.can_manage_invitees && author.can_close);
    assert!(
        !author.can_leave_request && !author.can_hold && !author.can_assess && !author.can_merge
    );

    let invitee = policy_for(&working, ViewerKind::Invitee).permissions;
    assert!(invitee.can_open_discussion && invitee.can_reply_to_discussion);
    assert!(invitee.can_pull_branch && invitee.can_push_branch && invitee.can_leave_request);
    assert!(!invitee.can_edit_description && !invitee.can_mark_ready);
    assert!(!invitee.can_manage_invitees && !invitee.can_close && !invitee.can_merge);

    let ready = ready_request();
    let unrelated = policy_for(&ready, ViewerKind::Unrelated).permissions;
    assert!(
        unrelated.can_open_discussion
            && unrelated.can_reply_to_discussion
            && unrelated.can_pull_branch
    );
    assert!(!unrelated.can_push_branch && !unrelated.can_edit_description);
    let author = policy_for(&ready, ViewerKind::Author).permissions;
    assert!(author.can_push_branch && author.can_return_to_working && author.can_manage_invitees);
    let maintainer = policy_for(&ready, ViewerKind::Maintainer).permissions;
    assert!(
        maintainer.can_push_branch
            && maintainer.can_hold
            && maintainer.can_assess
            && maintainer.can_merge
    );

    let held = held_request();
    for viewer in [ViewerKind::Author, ViewerKind::Invitee] {
        let permissions = policy_for(&held, viewer).permissions;
        assert!(
            permissions.can_open_discussion
                && permissions.can_reply_to_discussion
                && permissions.can_pull_branch
        );
        assert!(!permissions.can_push_branch && !permissions.can_edit_description);
        assert!(!permissions.can_return_to_working && !permissions.can_manage_invitees);
        assert_eq!(
            permissions.can_leave_request,
            matches!(viewer, ViewerKind::Invitee)
        );
    }
    let maintainer = policy_for(&held, ViewerKind::Maintainer).permissions;
    assert!(
        maintainer.can_push_branch
            && maintainer.can_edit_description
            && maintainer.can_manage_invitees
    );

    let private_working = private_request(working_request());
    let author = policy_for(&private_working, ViewerKind::Author).permissions;
    assert!(author.can_mark_ready && author.can_close && !author.can_manage_invitees);
    let private_held = private_request(held_request());
    assert!(
        policy_for(&private_held, ViewerKind::Author)
            .permissions
            .can_push_branch
    );
    assert!(
        policy_for(&private_held, ViewerKind::Maintainer)
            .permissions
            .can_push_branch
    );

    let completed = completed_request(RequestAssessmentOutcome::Neutral);
    for viewer in ViewerKind::ALL {
        let decision = policy_for(&completed, viewer);
        assert_eq!(
            decision.permissions.can_pull_branch,
            decision.request_ref_readable
        );
        assert!(!decision.permissions.can_push_branch);
        assert_eq!(
            decision.permissions.can_open_discussion,
            !matches!(viewer, ViewerKind::Anonymous)
        );
        assert_eq!(
            decision.permissions.can_reply_to_discussion,
            !matches!(viewer, ViewerKind::Anonymous)
        );
        assert!(!decision.permissions.can_manage_invitees);
    }
}

#[test]
fn hold_blocks_contributors_but_not_maintainers_and_completion_blocks_all() {
    let request = ready_request();
    assert!(
        policy_for(&request, ViewerKind::Author)
            .permissions
            .can_edit_description
    );
    let request = held_request();
    let author = policy_for(&request, ViewerKind::Author).permissions;
    assert!(!author.can_push_branch);
    assert!(!author.can_edit_description);
    assert!(!author.can_return_to_working);
    assert!(!author.can_manage_invitees);
    let maintainer = policy_for(&request, ViewerKind::Maintainer).permissions;
    assert!(maintainer.can_hold);
    assert!(maintainer.can_push_branch);
    assert!(maintainer.can_edit_description);
    assert!(maintainer.can_merge);
    assert!(maintainer.can_assess);
    assert!(maintainer.can_manage_invitees);
    assert_eq!(
        request_mergeability(&request, maintainer_access()).status,
        RequestMergeabilityStatus::Ready
    );

    let private_request = private_request(request);
    let maintainer = policy_for(&private_request, ViewerKind::Maintainer).permissions;
    assert!(!maintainer.can_manage_invitees);

    let request = completed_request(RequestAssessmentOutcome::Neutral);
    let maintainer = policy_for(&request, ViewerKind::Maintainer).permissions;
    assert!(maintainer.can_pull_branch);
    assert!(!maintainer.can_push_branch);
    assert!(maintainer.can_open_discussion);
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
fn ready_request_rejects_revisions_with_a_user_facing_constraint() {
    let request = ready_request();
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let error = record_request_revision(
        &mut requests,
        &mut BTreeMap::new(),
        RecordRequestRevisionInput {
            request_id: "request_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: Some("head".to_string()),
            new_head_oid: "head-2".to_string(),
            git_snapshot: source_blob("head-2"),
            event_id: "event_revision".to_string(),
            body: None,
            now_unix: 22,
        },
    )
    .unwrap_err();
    assert_eq!(
        error.message,
        "only working requests can receive new revisions"
    );
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
    assert!(
        policy_for(&request, ViewerKind::Author)
            .permissions
            .can_close
    );
    assert!(
        !policy_for(&request, ViewerKind::Maintainer)
            .permissions
            .can_close
    );
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

#[derive(Clone, Copy, Debug)]
enum ViewerKind {
    Anonymous,
    Unrelated,
    Author,
    Invitee,
    Maintainer,
}

impl ViewerKind {
    const ALL: [Self; 5] = [
        Self::Anonymous,
        Self::Unrelated,
        Self::Author,
        Self::Invitee,
        Self::Maintainer,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExpectedSurfaceDecision {
    listable: bool,
    exact_visible: bool,
    discussion_visible: bool,
    activity_stream_visible: bool,
    git_advertised: bool,
    request_ref_readable: bool,
    branch_mutable: bool,
    counts_as_ready: bool,
}

fn policy_for(request: &Request, viewer: ViewerKind) -> RequestPolicyDecision {
    let access = match viewer {
        ViewerKind::Maintainer => maintainer_access(),
        ViewerKind::Author if request.audience == RequestAudience::Private => maintainer_access(),
        _ => RepositoryAccess::public(),
    };
    let (user_id, is_invitee) = match viewer {
        ViewerKind::Anonymous => (None, false),
        ViewerKind::Unrelated => (Some("unrelated"), false),
        ViewerKind::Author => (Some(request.author_user_id.as_str()), false),
        ViewerKind::Invitee => (Some("invitee"), true),
        ViewerKind::Maintainer => (Some("maintainer"), false),
    };
    request_policy(request, RequestViewer::new(access, user_id, is_invitee))
}

fn assert_surface_decision(
    request_label: &str,
    viewer: ViewerKind,
    actual: RequestPolicyDecision,
    expected: ExpectedSurfaceDecision,
) {
    let actual = ExpectedSurfaceDecision {
        listable: actual.listable,
        exact_visible: actual.exact_visible,
        discussion_visible: actual.discussion_visible,
        activity_stream_visible: actual.activity_stream_visible,
        git_advertised: actual.git_advertised,
        request_ref_readable: actual.request_ref_readable,
        branch_mutable: actual.branch_mutable,
        counts_as_ready: actual.counts_as_ready,
    };
    assert_eq!(actual, expected, "{request_label} / {viewer:?}");
}

fn hidden() -> ExpectedSurfaceDecision {
    ExpectedSurfaceDecision {
        listable: false,
        exact_visible: false,
        discussion_visible: false,
        activity_stream_visible: false,
        git_advertised: false,
        request_ref_readable: false,
        branch_mutable: false,
        counts_as_ready: false,
    }
}

fn collaborator_working() -> ExpectedSurfaceDecision {
    ExpectedSurfaceDecision {
        listable: true,
        exact_visible: true,
        discussion_visible: true,
        activity_stream_visible: true,
        git_advertised: true,
        request_ref_readable: true,
        branch_mutable: true,
        counts_as_ready: false,
    }
}

fn exact_only_working() -> ExpectedSurfaceDecision {
    ExpectedSurfaceDecision {
        listable: false,
        exact_visible: true,
        discussion_visible: true,
        activity_stream_visible: false,
        git_advertised: false,
        request_ref_readable: true,
        branch_mutable: true,
        counts_as_ready: false,
    }
}

fn published_working_reader(branch_mutable: bool) -> ExpectedSurfaceDecision {
    ExpectedSurfaceDecision {
        listable: false,
        exact_visible: true,
        discussion_visible: true,
        activity_stream_visible: true,
        git_advertised: false,
        request_ref_readable: true,
        branch_mutable,
        counts_as_ready: false,
    }
}

fn published_ready_reader(branch_mutable: bool) -> ExpectedSurfaceDecision {
    ExpectedSurfaceDecision {
        listable: true,
        exact_visible: true,
        discussion_visible: true,
        activity_stream_visible: true,
        git_advertised: true,
        request_ref_readable: true,
        branch_mutable,
        counts_as_ready: true,
    }
}

fn published_history() -> ExpectedSurfaceDecision {
    ExpectedSurfaceDecision {
        listable: true,
        exact_visible: true,
        discussion_visible: true,
        activity_stream_visible: true,
        git_advertised: true,
        request_ref_readable: true,
        branch_mutable: false,
        counts_as_ready: false,
    }
}

fn private_visible(counts_as_ready: bool, branch_mutable: bool) -> ExpectedSurfaceDecision {
    ExpectedSurfaceDecision {
        listable: true,
        exact_visible: true,
        discussion_visible: true,
        activity_stream_visible: true,
        git_advertised: true,
        request_ref_readable: true,
        branch_mutable,
        counts_as_ready,
    }
}

fn no_permissions() -> RequestPermissions {
    RequestPermissions {
        can_open_discussion: false,
        can_reply_to_discussion: false,
        can_edit_description: false,
        can_pull_branch: false,
        can_push_branch: false,
        can_mark_ready: false,
        can_return_to_working: false,
        can_manage_invitees: false,
        can_leave_request: false,
        can_hold: false,
        can_assess: false,
        can_close: false,
        can_merge: false,
    }
}

fn published_working_request() -> Request {
    let mut request = ready_request();
    request.state = RequestState::Working;
    request.ready_at_unix = None;
    request.current_stake_credits = 0;
    request.validate_facts().unwrap();
    request
}

fn held_request() -> Request {
    let mut request = ready_request();
    request.held_at_unix = Some(21);
    request.held_by_user_id = Some("maintainer".to_string());
    request.updated_at_unix = 21;
    request.validate_facts().unwrap();
    request
}

fn private_request(mut request: Request) -> Request {
    request.audience = RequestAudience::Private;
    request.author_role = RequestActorRole::Member;
    request.current_stake_credits = 0;
    request.validate_facts().unwrap();
    request
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
    request.ready_queue_version = Some(1);
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
