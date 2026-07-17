use super::requests::*;
use crate::domain::store::{DEFAULT_GIT_FILE_MODE, RepositoryAccess, RepositoryActor, SourceBlob};
use crate::error::ApiError;
use std::collections::BTreeMap;

#[test]
fn request_policy_keeps_public_visibility_and_maintainer_decisions_in_domain() {
    let mut request = submitted_request();
    request.git_snapshot = None;
    assert!(request_visible_to_access(
        &request,
        RepositoryAccess::public()
    ));
    let anonymous = request_permissions(&request, RepositoryAccess::public(), None);
    assert!(anonymous.can_pull_branch);
    assert!(!anonymous.can_push_branch);
    assert!(!anonymous.can_open_discussion);
    assert!(!anonymous.can_merge);

    let contributor = request_permissions(
        &request,
        RepositoryAccess::public(),
        Some("another-contributor"),
    );
    assert!(contributor.can_pull_branch);
    assert!(contributor.can_push_branch);
    assert!(contributor.can_open_discussion);
    assert!(!contributor.can_merge);

    let maintainer = maintainer_access();
    assert!(request_permissions(&request, maintainer, Some("maintainer")).can_merge);
    assert_eq!(
        request_mergeability(&request, maintainer).status,
        RequestMergeabilityStatus::MissingRequestBranch
    );
    assert_eq!(
        request_actor_role(maintainer_access()),
        RequestActorRole::Member
    );
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

#[test]
fn invalid_credit_grants_do_not_mutate_accounts() {
    for (id, amount, message) in [
        ("ledger_grant", i32::MAX as u32 + 1, "exceeds i32 range"),
        ("repo_delete_refund:grant", 10, "working internal prefix"),
    ] {
        let mut accounts = BTreeMap::from([(
            "user_public".to_string(),
            UserCreditAccount {
                user_id: "user_public".to_string(),
                balance_credits: 20,
            },
        )]);
        let mut ledger_entries = BTreeMap::new();
        let error = grant_user_credits(
            &mut accounts,
            &mut ledger_entries,
            GrantUserCreditsInput {
                ledger_entry_id: id.to_string(),
                user_id: "user_public".to_string(),
                amount_credits: amount,
                now_unix: 10,
            },
        )
        .unwrap_err();
        assert!(error.message.contains(message));
        assert_eq!(accounts["user_public"].balance_credits, 20);
        assert!(ledger_entries.is_empty());
    }
}

#[test]
fn duplicate_request_name_in_same_repo_is_rejected_before_start() {
    let mut existing = submitted_request();
    existing.name = "fix-parser".to_string();
    let mut requests = BTreeMap::from([("req_1".to_string(), existing)]);
    let mut input = public_start_input();
    input.id = "req_2".to_string();
    input.name = "fix-parser".to_string();

    let error = start_request(&mut requests, input).unwrap_err();

    assert!(error.message.contains("request name already exists"));
    assert!(!requests.contains_key("req_2"));
}

#[test]
fn request_name_is_unique_per_repository_and_derives_branch_ref() {
    let mut existing = submitted_request();
    existing.name = "fix-parser".to_string();
    let mut requests = BTreeMap::from([("req_1".to_string(), existing)]);
    let mut input = public_start_input();
    input.id = "req_2".to_string();
    input.repo_id = "another/repo".to_string();
    input.name = "fix-parser".to_string();

    let mutation = start_request(&mut requests, input).unwrap();

    assert_eq!(mutation.request.name, "fix-parser");
    assert_eq!(
        canonical_request_ref(&mutation.request.name),
        "refs/heads/fix-parser"
    );
}

#[test]
fn omitted_title_defaults_to_request_name() {
    let mut input = public_start_input();
    input.title = None;

    let request = start_request(&mut BTreeMap::new(), input).unwrap().request;

    assert_eq!(request.title, "fix-parser");
}

#[test]
fn request_names_reject_reserved_and_git_unsafe_values() {
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
        let error = start_request(&mut BTreeMap::new(), input).unwrap_err();
        assert!(
            error.message.contains("request name"),
            "unexpected error for {invalid}: {}",
            error.message
        );
    }
}

#[test]
fn closed_requests_remain_readable_but_are_never_writable() {
    let mut request = submitted_request();
    request.state = RequestState::Resolved;
    for (access, viewer) in [
        (RepositoryAccess::public(), None),
        (RepositoryAccess::public(), Some("contributor")),
        (maintainer_access(), Some("maintainer")),
    ] {
        let permissions = request_permissions(&request, access, viewer);
        assert!(permissions.can_pull_branch);
        assert!(!permissions.can_push_branch);
        assert!(!permissions.can_open_discussion);
    }
}

#[test]
fn private_requests_are_invisible_to_public_access_and_writable_by_maintainers() {
    let mut request = submitted_request();
    request.audience = RequestAudience::Private;

    assert!(!request_visible_to_access(
        &request,
        RepositoryAccess::public()
    ));
    let public = request_permissions(&request, RepositoryAccess::public(), Some("contributor"));
    assert!(!public.can_pull_branch);
    assert!(!public.can_push_branch);

    let maintainer = request_permissions(&request, maintainer_access(), Some("maintainer"));
    assert!(maintainer.can_pull_branch);
    assert!(maintainer.can_push_branch);
}

#[test]
fn invalid_stakes_do_not_debit_accounts() {
    for (balance, stake) in [(u32::MAX, i32::MAX as u32 + 1), (i32::MAX as u32, 10)] {
        let mut fixture = RequestFixture::working(balance);
        let mut input = public_submit_input();
        input.stake_credits = stake;
        let error = fixture.submit(input).unwrap_err();
        assert!(error.message.contains("exceeds i32 range"));
        fixture.assert_unchanged(RequestState::Working, balance);
    }
}

#[test]
fn owner_submission_rejects_credit_stake() {
    let mut requests = BTreeMap::new();
    let mut events = BTreeMap::new();
    let mut input = public_start_input();
    input.author_role = RequestActorRole::Owner;
    input.audience = RequestAudience::Private;
    start_request(&mut requests, input).unwrap();
    record_working_request_upload(&mut requests, public_upload_input()).unwrap();
    let mut submit_input = public_submit_input();
    submit_input.stake_credits = 10;

    let error = submit_request(
        &mut requests,
        &mut events,
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        submit_input,
    )
    .unwrap_err();

    assert!(error.message.contains("do not use credit stake"));
}

#[test]
fn revision_reopens_needs_response_request() {
    let mut requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    requests.get_mut("req_1").unwrap().state = RequestState::NeedsResponse;
    let mut events = BTreeMap::new();

    let mutation =
        record_request_revision(&mut requests, &mut events, revision_input("head")).unwrap();

    assert_eq!(mutation.request.state, RequestState::Submitted);
    assert!(matches!(
        mutation.event.payload,
        RequestEventPayload::RevisionPushed {
            ref old_head_oid,
            ref new_head_oid,
            ..
        } if old_head_oid == "head" && new_head_oid == "new_head"
    ));
}

#[test]
fn revision_rejects_stale_expected_head() {
    let mut requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    let mut events = BTreeMap::new();

    let error = record_request_revision(&mut requests, &mut events, revision_input("stale_head"))
        .unwrap_err();

    assert!(error.message.contains("fetch and retry"));
    assert_eq!(requests.get("req_1").unwrap().head_oid, "head");
    assert!(events.is_empty());
}

#[test]
fn accepted_resolution_requires_merge_flow() {
    let mut fixture = RequestFixture::submitted(0);

    let error = fixture
        .resolve(resolve_input(RequestDisposition::Accepted))
        .unwrap_err();

    assert!(error.message.contains("merge flow"));
    fixture.assert_unchanged(RequestState::Submitted, 0);
}

#[test]
fn working_request_cannot_enter_maintainer_decision_flow() {
    let working_request = {
        let mut request = submitted_request();
        request.state = RequestState::Working;
        request.stake_credits = 0;
        request
    };

    let mut needs_fixture = RequestFixture::default();
    needs_fixture
        .requests
        .insert("req_1".to_string(), working_request.clone());
    let needs_response_error = mark_request_needs_response(
        &mut needs_fixture.requests,
        &mut needs_fixture.events,
        MarkRequestNeedsResponseInput {
            request_id: "req_1".to_string(),
            actor_user_id: "maintainer".to_string(),
            event_id: "event_needs_response".to_string(),
            body: "Please add tests.".to_string(),
            now_unix: 20,
        },
    )
    .unwrap_err();
    assert!(needs_response_error.message.contains("submitted"));

    let mut decision_fixture = RequestFixture::default();
    decision_fixture
        .requests
        .insert("req_1".to_string(), working_request);
    let resolve_error = decision_fixture
        .resolve(resolve_input(RequestDisposition::UsefulNotMerged))
        .unwrap_err();
    assert!(resolve_error.message.contains("submitted"));

    let merge_error = decision_fixture.merge(clean_merge_input()).unwrap_err();
    assert!(merge_error.message.contains("submitted"));
    assert_eq!(
        decision_fixture.requests["req_1"].state,
        RequestState::Working
    );
    assert!(decision_fixture.events.is_empty());
    assert!(decision_fixture.ledger_entries.is_empty());
}

#[test]
fn clean_merge_accepts_and_settles_public_request() {
    let mut fixture = RequestFixture::submitted(0);

    let mutation = fixture.merge(clean_merge_input()).unwrap();

    assert_eq!(mutation.request.state, RequestState::Resolved);
    assert_eq!(
        mutation.request.disposition,
        Some(RequestDisposition::Accepted)
    );
    assert_eq!(mutation.request.settlement.unwrap().reward_credits, 5);
    assert_eq!(mutation.merged_event.kind, RequestEventKind::Merged);
    assert_eq!(mutation.settled_event.kind, RequestEventKind::Settled);
    assert_eq!(fixture.accounts["user_public"].balance_credits, 15);
    assert_eq!(mutation.ledger_entries.len(), 2);
}

#[test]
fn clean_merge_rejects_stale_inputs_without_settling() {
    for (input, message) in [
        (
            {
                let mut input = clean_merge_input();
                input.current_main_oid = "new-main".to_string();
                input
            },
            "main changed",
        ),
        (
            {
                let mut input = clean_merge_input();
                input.expected_head_oid = "old-head".to_string();
                input
            },
            "request changed",
        ),
    ] {
        let mut fixture = RequestFixture::submitted(0);
        let error = fixture.merge(input).unwrap_err();
        assert!(error.message.contains(message));
        fixture.assert_unchanged(RequestState::Submitted, 0);
    }
}

#[test]
fn owner_clean_merge_does_not_touch_credit_accounts() {
    let mut request = submitted_request();
    request.author_user_id = "user_owner".to_string();
    request.author_role = RequestActorRole::Owner;
    request.audience = RequestAudience::Private;
    request.stake_credits = 0;
    let mut fixture = RequestFixture::default();
    fixture.requests.insert("req_1".to_string(), request);

    let mutation = fixture.merge(clean_merge_input()).unwrap();

    assert_eq!(
        mutation.request.disposition,
        Some(RequestDisposition::Accepted)
    );
    assert!(mutation.account.is_none());
    assert!(mutation.ledger_entries.is_empty());
    assert!(fixture.accounts.is_empty());
    assert!(fixture.ledger_entries.is_empty());
}

#[test]
fn duplicate_settlement_ledger_ids_do_not_mutate_request_or_account() {
    let mut fixture = RequestFixture::submitted(0);

    let mut input = resolve_input(RequestDisposition::UsefulNotMerged);
    input.refund_ledger_entry_id = Some("ledger_settle".to_string());
    input.reward_ledger_entry_id = Some("ledger_settle".to_string());
    let error = fixture.resolve(input).unwrap_err();

    assert!(error.message.contains("must be unique"));
    fixture.assert_unchanged(RequestState::Submitted, 0);
}

#[test]
fn abandonment_requires_contributor_turn() {
    let mut fixture = RequestFixture::submitted(0);

    let error = fixture
        .resolve(resolve_input(RequestDisposition::Abandoned))
        .unwrap_err();

    assert!(error.message.contains("waiting on the contributor"));
}

#[test]
fn settlement_cannot_run_twice() {
    let mut request = submitted_request();
    request.state = RequestState::Resolved;
    request.settlement = Some(settlement_for(10, RequestDisposition::LowQuality, 20));
    let mut fixture = RequestFixture::default();
    fixture.requests.insert("req_1".to_string(), request);

    let error = fixture
        .resolve(resolve_input(RequestDisposition::Accepted))
        .unwrap_err();

    assert!(error.message.contains("already closed"));
}

#[test]
fn discussions_are_append_only_positioned_and_read_by_their_author() {
    let request = submitted_request();
    let starting_version = request.activity_version;
    let starting_updated_at = request.updated_at_unix;
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let mut discussions = BTreeMap::new();
    let mutation = create_request_discussion(
        &mut requests,
        &mut discussions,
        CreateRequestDiscussionInput {
            request_id: "req_1".to_string(),
            id: "discussion_1".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_participate: true,
            client_discussion_id: "client_1".to_string(),
            body_markdown: "Parser ownership".to_string(),
            now_unix: 30,
        },
    )
    .unwrap();

    assert_eq!(mutation.discussion.opened_position, starting_version + 1);
    assert_eq!(
        mutation.read_state.read_through_position,
        mutation.discussion.opened_position
    );
    assert_eq!(mutation.request.activity_version, starting_version + 1);
    assert_eq!(mutation.request.updated_at_unix, starting_updated_at);
}

#[test]
fn resolved_discussion_requires_atomic_reopen_and_reply() {
    let request = submitted_request();
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let mut discussions = BTreeMap::new();
    let root = create_request_discussion(
        &mut requests,
        &mut discussions,
        CreateRequestDiscussionInput {
            request_id: "req_1".to_string(),
            id: "discussion_1".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_participate: true,
            client_discussion_id: "client_1".to_string(),
            body_markdown: "Parser ownership".to_string(),
            now_unix: 30,
        },
    )
    .unwrap();
    let resolved = resolve_request_discussion(
        &mut requests,
        &mut discussions,
        ResolveRequestDiscussionInput {
            request_id: "req_1".to_string(),
            discussion_id: root.discussion.id.clone(),
            actor_user_id: "maintainer".to_string(),
            actor_is_maintainer: true,
            event_id: "event_resolved_discussion".to_string(),
            now_unix: 31,
        },
    )
    .unwrap();
    assert_eq!(
        resolved.event.position,
        resolved.discussion.last_activity_position
    );

    let mut replies = BTreeMap::new();
    let ordinary = create_request_discussion_reply(
        &mut requests,
        &mut discussions,
        &mut replies,
        CreateRequestDiscussionReplyInput {
            request_id: "req_1".to_string(),
            discussion_id: root.discussion.id.clone(),
            id: "reply_rejected".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_participate: true,
            client_reply_id: "client_rejected".to_string(),
            body_markdown: "One more point".to_string(),
            reply_to_reply_id: None,
            now_unix: 32,
        },
    )
    .unwrap_err();
    assert_eq!(ordinary.kind, crate::error::ErrorKind::Conflict);

    let reopened = reopen_and_reply_to_request_discussion(
        &mut requests,
        &mut discussions,
        &mut replies,
        ReopenAndReplyToRequestDiscussionInput {
            request_id: "req_1".to_string(),
            discussion_id: root.discussion.id,
            reply_id: "reply_1".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_is_maintainer: false,
            actor_can_participate: true,
            event_id: "event_reopened_discussion".to_string(),
            client_reply_id: "client_reply_1".to_string(),
            body_markdown: "One more point".to_string(),
            reply_to_reply_id: None,
            now_unix: 33,
        },
    )
    .unwrap();
    assert_eq!(reopened.discussion.status, RequestDiscussionStatus::Open);
    assert_eq!(
        reopened.reply.position,
        reopened.discussion.last_activity_position
    );
    assert_eq!(
        reopened.activity_event.as_ref().unwrap().position,
        reopened.reply.position
    );
}

#[test]
fn discussion_read_markers_are_monotonic_and_bodies_are_bounded() {
    let request = submitted_request();
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let mut discussions = BTreeMap::new();
    let root = create_request_discussion(
        &mut requests,
        &mut discussions,
        CreateRequestDiscussionInput {
            request_id: "req_1".to_string(),
            id: "discussion_1".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_participate: true,
            client_discussion_id: "client_1".to_string(),
            body_markdown: "Parser ownership".to_string(),
            now_unix: 30,
        },
    )
    .unwrap();
    let mut reads = BTreeMap::new();
    let latest = mark_request_discussion_read(
        &discussions,
        &mut reads,
        MarkRequestDiscussionReadInput {
            discussion_id: root.discussion.id.clone(),
            user_id: "maintainer".to_string(),
            through_position: root.discussion.last_activity_position,
            now_unix: 31,
        },
    )
    .unwrap();
    let stale = mark_request_discussion_read(
        &discussions,
        &mut reads,
        MarkRequestDiscussionReadInput {
            discussion_id: root.discussion.id,
            user_id: "maintainer".to_string(),
            through_position: 0,
            now_unix: 32,
        },
    )
    .unwrap();
    assert_eq!(stale, latest);

    let oversized = create_request_discussion(
        &mut requests,
        &mut discussions,
        CreateRequestDiscussionInput {
            request_id: "req_1".to_string(),
            id: "discussion_oversized".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_participate: true,
            client_discussion_id: "client_oversized".to_string(),
            body_markdown: "x".repeat(REQUEST_DISCUSSION_BODY_MAX_BYTES + 1),
            now_unix: 33,
        },
    )
    .unwrap_err();
    assert_eq!(oversized.kind, crate::error::ErrorKind::BadRequest);

    let oversized_client_id = create_request_discussion(
        &mut requests,
        &mut discussions,
        CreateRequestDiscussionInput {
            request_id: "req_1".to_string(),
            id: "discussion_oversized_client_id".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_participate: true,
            client_discussion_id: "x".repeat(REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES + 1),
            body_markdown: "Bound the idempotency key".to_string(),
            now_unix: 34,
        },
    )
    .unwrap_err();
    assert_eq!(
        oversized_client_id.kind,
        crate::error::ErrorKind::BadRequest
    );

    let oversized_reply_client_id = create_request_discussion_reply(
        &mut requests,
        &mut discussions,
        &mut BTreeMap::new(),
        CreateRequestDiscussionReplyInput {
            request_id: "req_1".to_string(),
            discussion_id: "discussion_1".to_string(),
            id: "reply_oversized_client_id".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_participate: true,
            client_reply_id: "x".repeat(REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES + 1),
            body_markdown: "Bound the reply idempotency key".to_string(),
            reply_to_reply_id: None,
            now_unix: 35,
        },
    )
    .unwrap_err();
    assert_eq!(
        oversized_reply_client_id.kind,
        crate::error::ErrorKind::BadRequest
    );
}

#[test]
fn discussion_authority_and_activity_are_independent_from_request_lifecycle() {
    let mut request = submitted_request();
    request.state = RequestState::NeedsResponse;
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let mut discussions = BTreeMap::new();
    let root = create_request_discussion(
        &mut requests,
        &mut discussions,
        CreateRequestDiscussionInput {
            request_id: "req_1".to_string(),
            id: "discussion_1".to_string(),
            actor_user_id: "topic_opener".to_string(),
            actor_can_participate: true,
            client_discussion_id: "client_1".to_string(),
            body_markdown: "Parser ownership".to_string(),
            now_unix: 30,
        },
    )
    .unwrap();
    assert_eq!(requests["req_1"].state, RequestState::NeedsResponse);

    let unrelated = resolve_request_discussion(
        &mut requests,
        &mut discussions,
        ResolveRequestDiscussionInput {
            request_id: "req_1".to_string(),
            discussion_id: root.discussion.id.clone(),
            actor_user_id: "unrelated".to_string(),
            actor_is_maintainer: false,
            event_id: "event_forbidden".to_string(),
            now_unix: 31,
        },
    )
    .unwrap_err();
    assert_eq!(unrelated.kind, crate::error::ErrorKind::Forbidden);

    let author_resolved = resolve_request_discussion(
        &mut requests,
        &mut discussions,
        ResolveRequestDiscussionInput {
            request_id: "req_1".to_string(),
            discussion_id: root.discussion.id.clone(),
            actor_user_id: "user_public".to_string(),
            actor_is_maintainer: false,
            event_id: "event_author_resolved".to_string(),
            now_unix: 32,
        },
    )
    .unwrap();
    assert_eq!(
        author_resolved.event.kind,
        RequestEventKind::DiscussionResolved
    );
    assert_eq!(requests["req_1"].state, RequestState::NeedsResponse);

    let opener_reopened = reopen_request_discussion(
        &mut requests,
        &mut discussions,
        ReopenRequestDiscussionInput {
            request_id: "req_1".to_string(),
            discussion_id: root.discussion.id,
            actor_user_id: "topic_opener".to_string(),
            actor_is_maintainer: false,
            event_id: "event_opener_reopened".to_string(),
            now_unix: 33,
        },
    )
    .unwrap();
    assert_eq!(
        opener_reopened.event.kind,
        RequestEventKind::DiscussionReopened
    );
    assert_eq!(requests["req_1"].state, RequestState::NeedsResponse);
}

#[test]
fn description_edits_are_authorized_bounded_and_typed() {
    let request = submitted_request();
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let mut events = BTreeMap::new();
    let forbidden = update_request_description(
        &mut requests,
        &mut events,
        UpdateRequestDescriptionInput {
            request_id: "req_1".to_string(),
            actor_user_id: "unrelated".to_string(),
            actor_can_edit_description: false,
            event_id: "event_forbidden".to_string(),
            description_markdown: "No".to_string(),
            now_unix: 30,
        },
    )
    .unwrap_err();
    assert_eq!(forbidden.kind, crate::error::ErrorKind::Forbidden);
    assert!(events.is_empty());

    let mutation = update_request_description(
        &mut requests,
        &mut events,
        UpdateRequestDescriptionInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_edit_description: true,
            event_id: "event_description".to_string(),
            description_markdown: "## Intent\nFix parsing.".to_string(),
            now_unix: 31,
        },
    )
    .unwrap();
    assert!(matches!(
        mutation.event.payload,
        RequestEventPayload::DescriptionEdited {
            ref new_markdown,
            ..
        } if new_markdown == "## Intent\nFix parsing."
    ));

    let oversized = update_request_description(
        &mut requests,
        &mut events,
        UpdateRequestDescriptionInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_edit_description: true,
            event_id: "event_oversized".to_string(),
            description_markdown: "x".repeat(REQUEST_DESCRIPTION_MAX_BYTES + 1),
            now_unix: 32,
        },
    )
    .unwrap_err();
    assert_eq!(oversized.kind, crate::error::ErrorKind::BadRequest);
}

#[derive(Default)]
struct RequestFixture {
    requests: BTreeMap<String, Request>,
    events: BTreeMap<String, RequestEvent>,
    accounts: BTreeMap<String, UserCreditAccount>,
    ledger_entries: BTreeMap<String, CreditLedgerEntry>,
}

impl RequestFixture {
    fn with_balance(balance_credits: u32) -> Self {
        Self {
            accounts: BTreeMap::from([(
                "user_public".to_string(),
                UserCreditAccount {
                    user_id: "user_public".to_string(),
                    balance_credits,
                },
            )]),
            ..Self::default()
        }
    }

    fn working(balance_credits: u32) -> Self {
        let mut fixture = Self::with_balance(balance_credits);
        start_request(&mut fixture.requests, public_start_input()).unwrap();
        record_working_request_upload(&mut fixture.requests, public_upload_input()).unwrap();
        fixture
    }

    fn submitted(balance_credits: u32) -> Self {
        let mut fixture = Self::with_balance(balance_credits);
        fixture
            .requests
            .insert("req_1".to_string(), submitted_request());
        fixture
    }

    fn submit(&mut self, input: SubmitRequestInput) -> Result<SubmitRequestMutation, ApiError> {
        submit_request(
            &mut self.requests,
            &mut self.events,
            &mut self.accounts,
            &mut self.ledger_entries,
            input,
        )
    }

    fn resolve(&mut self, input: ResolveRequestInput) -> Result<ResolveRequestMutation, ApiError> {
        resolve_request(
            &mut self.requests,
            &mut self.events,
            &mut self.accounts,
            &mut self.ledger_entries,
            input,
        )
    }

    fn merge(&mut self, input: MergeRequestInput) -> Result<MergeRequestMutation, ApiError> {
        merge_request(
            &mut self.requests,
            &mut self.events,
            &mut self.accounts,
            &mut self.ledger_entries,
            input,
        )
    }

    fn assert_unchanged(&self, state: RequestState, balance: u32) {
        assert_eq!(self.requests["req_1"].state, state);
        assert_eq!(self.accounts["user_public"].balance_credits, balance);
        assert!(self.events.is_empty());
        assert!(self.ledger_entries.is_empty());
    }
}

fn public_start_input() -> StartRequestInput {
    StartRequestInput {
        id: "req_1".to_string(),
        repo_id: "owner/repo".to_string(),
        name: "fix-parser".to_string(),
        author_user_id: "user_public".to_string(),
        title: Some("Fix parser crash".to_string()),
        author_role: RequestActorRole::Public,
        audience: RequestAudience::Public,
        base_main_oid: "base".to_string(),
        event_id: "event_started".to_string(),
        now_unix: 10,
    }
}

fn public_upload_input() -> RecordWorkingRequestUploadInput {
    RecordWorkingRequestUploadInput {
        request_id: "req_1".to_string(),
        actor_user_id: "user_public".to_string(),
        actor_can_edit: true,
        expected_old_head_oid: None,
        new_head_oid: "head".to_string(),
        git_snapshot: source_blob("head"),
        now_unix: 11,
    }
}

fn public_submit_input() -> SubmitRequestInput {
    SubmitRequestInput {
        request_id: "req_1".to_string(),
        actor_user_id: "user_public".to_string(),
        expected_head_oid: "head".to_string(),
        stake_credits: 10,
        stake_ledger_entry_id: Some("ledger_stake".to_string()),
        event_id: "event_created".to_string(),
        now_unix: 12,
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

fn clean_merge_input() -> MergeRequestInput {
    MergeRequestInput {
        request_id: "req_1".to_string(),
        actor_user_id: "maintainer".to_string(),
        expected_main_oid: "base".to_string(),
        current_main_oid: "base".to_string(),
        expected_head_oid: "head".to_string(),
        event_id: "event_merged".to_string(),
        settlement_event_id: "event_settled".to_string(),
        refund_ledger_entry_id: Some("ledger_refund".to_string()),
        reward_ledger_entry_id: Some("ledger_reward".to_string()),
        body: None,
        now_unix: 30,
    }
}

fn resolve_input(disposition: RequestDisposition) -> ResolveRequestInput {
    ResolveRequestInput {
        request_id: "req_1".to_string(),
        actor_user_id: "maintainer".to_string(),
        disposition,
        event_id: "event_resolved".to_string(),
        settlement_event_id: "event_settled".to_string(),
        refund_ledger_entry_id: Some("ledger_refund".to_string()),
        reward_ledger_entry_id: Some("ledger_reward".to_string()),
        body: None,
        now_unix: 30,
    }
}

fn revision_input(expected_head: &str) -> RecordRequestRevisionInput {
    RecordRequestRevisionInput {
        request_id: "req_1".to_string(),
        actor_user_id: "user_public".to_string(),
        actor_can_edit: true,
        expected_old_head_oid: Some(expected_head.to_string()),
        new_head_oid: "new_head".to_string(),
        git_snapshot: None,
        event_id: "event_revision".to_string(),
        body: None,
        now_unix: 20,
    }
}

fn submitted_request() -> Request {
    let mut fixture = RequestFixture::working(20);
    fixture.submit(public_submit_input()).unwrap();
    fixture.requests.remove("req_1").unwrap()
}
