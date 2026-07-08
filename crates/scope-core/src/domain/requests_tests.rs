use super::requests::*;
use std::collections::BTreeMap;

#[test]
fn public_submission_debits_stake_once() {
    let mut requests = BTreeMap::new();
    let mut events = BTreeMap::new();
    let mut accounts = BTreeMap::from([(
        "user_public".to_string(),
        UserCreditAccount {
            user_id: "user_public".to_string(),
            balance_credits: 20,
        },
    )]);
    let mut ledger_entries = BTreeMap::new();

    let mutation = submit_request(
        &mut requests,
        &mut events,
        &mut accounts,
        &mut ledger_entries,
        public_submit_input(),
    )
    .unwrap();

    assert_eq!(mutation.request.stake_credits, 10);
    assert_eq!(accounts.get("user_public").unwrap().balance_credits, 10);
    assert_eq!(
        mutation.ledger_entry.unwrap().kind,
        CreditLedgerEntryKind::RequestStakeDebit
    );
}

#[test]
fn credit_grant_failure_does_not_mutate_accounts() {
    let mut accounts = BTreeMap::new();
    let mut ledger_entries = BTreeMap::new();

    let error = grant_user_credits(
        &mut accounts,
        &mut ledger_entries,
        GrantUserCreditsInput {
            ledger_entry_id: "ledger_grant".to_string(),
            user_id: "user_public".to_string(),
            amount_credits: i32::MAX as u32 + 1,
            now_unix: 10,
        },
    )
    .unwrap_err();

    assert!(error.message.contains("exceeds i32 range"));
    assert!(accounts.is_empty());
    assert!(ledger_entries.is_empty());
}

#[test]
fn reserved_internal_ledger_prefixes_are_rejected() {
    let mut accounts = BTreeMap::from([(
        "user_public".to_string(),
        UserCreditAccount {
            user_id: "user_public".to_string(),
            balance_credits: 20,
        },
    )]);
    let mut ledger_entries = BTreeMap::new();

    let grant_error = grant_user_credits(
        &mut accounts,
        &mut ledger_entries,
        GrantUserCreditsInput {
            ledger_entry_id: "repo_delete_refund:grant".to_string(),
            user_id: "user_public".to_string(),
            amount_credits: 10,
            now_unix: 10,
        },
    )
    .unwrap_err();
    assert!(grant_error.message.contains("reserved internal prefix"));

    let mut submit_input = public_submit_input();
    submit_input.stake_ledger_entry_id = Some("repo_delete_refund:stake".to_string());
    let submit_error = submit_request(
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        &mut accounts,
        &mut ledger_entries,
        submit_input,
    )
    .unwrap_err();
    assert!(submit_error.message.contains("reserved internal prefix"));

    let mut requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    let resolve_error = resolve_request(
        &mut requests,
        &mut BTreeMap::new(),
        &mut accounts,
        &mut ledger_entries,
        ResolveRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "maintainer".to_string(),
            disposition: RequestDisposition::UsefulNotMerged,
            event_id: "event_resolved".to_string(),
            settlement_event_id: "event_settled".to_string(),
            refund_ledger_entry_id: Some("repo_delete_refund:settle".to_string()),
            reward_ledger_entry_id: Some("ledger_reward".to_string()),
            body: None,
            now_unix: 30,
        },
    )
    .unwrap_err();
    assert!(resolve_error.message.contains("reserved internal prefix"));

    assert!(ledger_entries.is_empty());
}

#[test]
fn duplicate_request_ref_is_rejected_before_credit_debit() {
    let mut existing = submitted_request();
    existing.request_ref = "refs/scope/requests/req_2".to_string();
    let mut requests = BTreeMap::from([("req_1".to_string(), existing)]);
    let mut events = BTreeMap::new();
    let mut accounts = BTreeMap::from([(
        "user_public".to_string(),
        UserCreditAccount {
            user_id: "user_public".to_string(),
            balance_credits: 20,
        },
    )]);
    let mut ledger_entries = BTreeMap::new();
    let mut input = public_submit_input();
    input.id = "req_2".to_string();
    input.request_ref = "refs/scope/requests/req_2".to_string();
    input.event_id = "event_created_2".to_string();
    input.stake_ledger_entry_id = Some("ledger_stake_2".to_string());

    let error = submit_request(
        &mut requests,
        &mut events,
        &mut accounts,
        &mut ledger_entries,
        input,
    )
    .unwrap_err();

    assert!(error.message.contains("request ref already exists"));
    assert!(!requests.contains_key("req_2"));
    assert!(events.is_empty());
    assert!(ledger_entries.is_empty());
    assert_eq!(accounts.get("user_public").unwrap().balance_credits, 20);
}

#[test]
fn invalid_stake_amount_does_not_debit_account() {
    let mut requests = BTreeMap::new();
    let mut events = BTreeMap::new();
    let mut accounts = BTreeMap::from([(
        "user_public".to_string(),
        UserCreditAccount {
            user_id: "user_public".to_string(),
            balance_credits: u32::MAX,
        },
    )]);
    let mut ledger_entries = BTreeMap::new();
    let mut input = public_submit_input();
    input.stake_credits = i32::MAX as u32 + 1;

    let error = submit_request(
        &mut requests,
        &mut events,
        &mut accounts,
        &mut ledger_entries,
        input,
    )
    .unwrap_err();

    assert!(error.message.contains("exceeds i32 range"));
    assert!(requests.is_empty());
    assert!(events.is_empty());
    assert!(ledger_entries.is_empty());
    assert_eq!(
        accounts.get("user_public").unwrap().balance_credits,
        u32::MAX
    );
}

#[test]
fn public_submission_reserves_room_for_maximum_reward() {
    let mut requests = BTreeMap::new();
    let mut events = BTreeMap::new();
    let mut accounts = BTreeMap::from([(
        "user_public".to_string(),
        UserCreditAccount {
            user_id: "user_public".to_string(),
            balance_credits: i32::MAX as u32,
        },
    )]);
    let mut ledger_entries = BTreeMap::new();

    let error = submit_request(
        &mut requests,
        &mut events,
        &mut accounts,
        &mut ledger_entries,
        public_submit_input(),
    )
    .unwrap_err();

    assert!(error.message.contains("exceeds i32 range"));
    assert!(requests.is_empty());
    assert!(events.is_empty());
    assert!(ledger_entries.is_empty());
    assert_eq!(
        accounts.get("user_public").unwrap().balance_credits,
        i32::MAX as u32
    );
}

#[test]
fn owner_submission_rejects_credit_stake() {
    let mut input = public_submit_input();
    input.author_role = RequestActorRole::Owner;
    input.base_audience = RequestBaseAudience::Private;

    let error = submit_request(
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        input,
    )
    .unwrap_err();

    assert!(error.message.contains("do not use credit stake"));
}

#[test]
fn revision_reopens_needs_response_request() {
    let mut requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    requests.get_mut("req_1").unwrap().state = RequestState::NeedsResponse;
    let mut events = BTreeMap::new();

    let mutation = record_request_revision(
        &mut requests,
        &mut events,
        RecordRequestRevisionInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            expected_old_head_oid: Some("head".to_string()),
            new_head_oid: "new_head".to_string(),
            git_snapshot: None,
            event_id: "event_revision".to_string(),
            body: None,
            now_unix: 20,
        },
    )
    .unwrap();

    assert_eq!(mutation.request.state, RequestState::Submitted);
    assert_eq!(mutation.event.old_head_oid.as_deref(), Some("head"));
    assert_eq!(mutation.event.new_head_oid.as_deref(), Some("new_head"));
}

#[test]
fn revision_rejects_stale_expected_head() {
    let mut requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    let mut events = BTreeMap::new();

    let error = record_request_revision(
        &mut requests,
        &mut events,
        RecordRequestRevisionInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            expected_old_head_oid: Some("stale_head".to_string()),
            new_head_oid: "new_head".to_string(),
            git_snapshot: None,
            event_id: "event_revision".to_string(),
            body: None,
            now_unix: 20,
        },
    )
    .unwrap_err();

    assert!(error.message.contains("fetch and retry"));
    assert_eq!(requests.get("req_1").unwrap().head_oid, "head");
    assert!(events.is_empty());
}

#[test]
fn useful_not_merged_settlement_refunds_and_rewards() {
    let mut requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    let mut events = BTreeMap::new();
    let mut accounts = BTreeMap::from([(
        "user_public".to_string(),
        UserCreditAccount {
            user_id: "user_public".to_string(),
            balance_credits: 0,
        },
    )]);
    let mut ledger_entries = BTreeMap::new();

    let mutation = resolve_request(
        &mut requests,
        &mut events,
        &mut accounts,
        &mut ledger_entries,
        ResolveRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "maintainer".to_string(),
            disposition: RequestDisposition::UsefulNotMerged,
            event_id: "event_resolved".to_string(),
            settlement_event_id: "event_settled".to_string(),
            refund_ledger_entry_id: Some("ledger_refund".to_string()),
            reward_ledger_entry_id: Some("ledger_reward".to_string()),
            body: None,
            now_unix: 30,
        },
    )
    .unwrap();

    assert_eq!(mutation.request.state, RequestState::Resolved);
    assert_eq!(mutation.request.settlement.unwrap().reward_credits, 2);
    assert_eq!(accounts.get("user_public").unwrap().balance_credits, 12);
    assert_eq!(mutation.ledger_entries.len(), 2);
}

#[test]
fn accepted_resolution_requires_merge_flow() {
    let mut requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    let mut events = BTreeMap::new();
    let mut accounts = BTreeMap::from([(
        "user_public".to_string(),
        UserCreditAccount {
            user_id: "user_public".to_string(),
            balance_credits: 0,
        },
    )]);
    let mut ledger_entries = BTreeMap::new();

    let error = resolve_request(
        &mut requests,
        &mut events,
        &mut accounts,
        &mut ledger_entries,
        ResolveRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "maintainer".to_string(),
            disposition: RequestDisposition::Accepted,
            event_id: "event_resolved".to_string(),
            settlement_event_id: "event_settled".to_string(),
            refund_ledger_entry_id: Some("ledger_refund".to_string()),
            reward_ledger_entry_id: Some("ledger_reward".to_string()),
            body: None,
            now_unix: 30,
        },
    )
    .unwrap_err();

    assert!(error.message.contains("merge flow"));
    assert_eq!(
        requests.get("req_1").unwrap().state,
        RequestState::Submitted
    );
    assert_eq!(accounts.get("user_public").unwrap().balance_credits, 0);
    assert!(events.is_empty());
    assert!(ledger_entries.is_empty());
}

#[test]
fn clean_merge_accepts_and_settles_public_request() {
    let mut requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    let mut events = BTreeMap::new();
    let mut accounts = BTreeMap::from([(
        "user_public".to_string(),
        UserCreditAccount {
            user_id: "user_public".to_string(),
            balance_credits: 0,
        },
    )]);
    let mut ledger_entries = BTreeMap::new();

    let mutation = merge_request(
        &mut requests,
        &mut events,
        &mut accounts,
        &mut ledger_entries,
        clean_merge_input(),
    )
    .unwrap();

    assert_eq!(mutation.request.state, RequestState::Resolved);
    assert_eq!(
        mutation.request.disposition,
        Some(RequestDisposition::Accepted)
    );
    assert_eq!(mutation.request.settlement.unwrap().reward_credits, 5);
    assert_eq!(mutation.merged_event.kind, RequestEventKind::Merged);
    assert_eq!(mutation.settled_event.kind, RequestEventKind::Settled);
    assert_eq!(accounts.get("user_public").unwrap().balance_credits, 15);
    assert_eq!(mutation.ledger_entries.len(), 2);
}

#[test]
fn clean_merge_rejects_stale_main_without_settling() {
    let mut requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    let mut events = BTreeMap::new();
    let mut accounts = BTreeMap::from([(
        "user_public".to_string(),
        UserCreditAccount {
            user_id: "user_public".to_string(),
            balance_credits: 0,
        },
    )]);
    let mut ledger_entries = BTreeMap::new();
    let mut input = clean_merge_input();
    input.current_main_oid = "new-main".to_string();

    let error = merge_request(
        &mut requests,
        &mut events,
        &mut accounts,
        &mut ledger_entries,
        input,
    )
    .unwrap_err();

    assert!(error.message.contains("main changed"));
    assert_eq!(
        requests.get("req_1").unwrap().state,
        RequestState::Submitted
    );
    assert!(events.is_empty());
    assert!(ledger_entries.is_empty());
    assert_eq!(accounts.get("user_public").unwrap().balance_credits, 0);
}

#[test]
fn clean_merge_rejects_stale_request_head_without_settling() {
    let mut requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    let mut events = BTreeMap::new();
    let mut accounts = BTreeMap::from([(
        "user_public".to_string(),
        UserCreditAccount {
            user_id: "user_public".to_string(),
            balance_credits: 0,
        },
    )]);
    let mut ledger_entries = BTreeMap::new();
    let mut input = clean_merge_input();
    input.expected_head_oid = "old-head".to_string();

    let error = merge_request(
        &mut requests,
        &mut events,
        &mut accounts,
        &mut ledger_entries,
        input,
    )
    .unwrap_err();

    assert!(error.message.contains("request changed"));
    assert_eq!(
        requests.get("req_1").unwrap().state,
        RequestState::Submitted
    );
    assert!(events.is_empty());
    assert!(ledger_entries.is_empty());
    assert_eq!(accounts.get("user_public").unwrap().balance_credits, 0);
}

#[test]
fn owner_clean_merge_does_not_touch_credit_accounts() {
    let mut request = submitted_request();
    request.author_user_id = "user_owner".to_string();
    request.author_role = RequestActorRole::Owner;
    request.base_audience = RequestBaseAudience::Private;
    request.stake_credits = 0;
    let mut requests = BTreeMap::from([("req_1".to_string(), request)]);
    let mut events = BTreeMap::new();
    let mut accounts = BTreeMap::new();
    let mut ledger_entries = BTreeMap::new();

    let mutation = merge_request(
        &mut requests,
        &mut events,
        &mut accounts,
        &mut ledger_entries,
        clean_merge_input(),
    )
    .unwrap();

    assert_eq!(
        mutation.request.disposition,
        Some(RequestDisposition::Accepted)
    );
    assert!(mutation.account.is_none());
    assert!(mutation.ledger_entries.is_empty());
    assert!(accounts.is_empty());
    assert!(ledger_entries.is_empty());
}

#[test]
fn duplicate_settlement_ledger_ids_do_not_mutate_request_or_account() {
    let mut requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    let mut events = BTreeMap::new();
    let mut accounts = BTreeMap::from([(
        "user_public".to_string(),
        UserCreditAccount {
            user_id: "user_public".to_string(),
            balance_credits: 0,
        },
    )]);
    let mut ledger_entries = BTreeMap::new();

    let error = resolve_request(
        &mut requests,
        &mut events,
        &mut accounts,
        &mut ledger_entries,
        ResolveRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "maintainer".to_string(),
            disposition: RequestDisposition::UsefulNotMerged,
            event_id: "event_resolved".to_string(),
            settlement_event_id: "event_settled".to_string(),
            refund_ledger_entry_id: Some("ledger_settle".to_string()),
            reward_ledger_entry_id: Some("ledger_settle".to_string()),
            body: None,
            now_unix: 30,
        },
    )
    .unwrap_err();

    assert!(error.message.contains("must be unique"));
    assert_eq!(
        requests.get("req_1").unwrap().state,
        RequestState::Submitted
    );
    assert_eq!(accounts.get("user_public").unwrap().balance_credits, 0);
    assert!(events.is_empty());
    assert!(ledger_entries.is_empty());
}

#[test]
fn settlement_credit_conversion_failure_does_not_mutate_request_or_account() {
    let mut request = submitted_request();
    request.stake_credits = i32::MAX as u32 + 1;
    let mut requests = BTreeMap::from([("req_1".to_string(), request)]);
    let mut events = BTreeMap::new();
    let mut accounts = BTreeMap::from([(
        "user_public".to_string(),
        UserCreditAccount {
            user_id: "user_public".to_string(),
            balance_credits: 0,
        },
    )]);
    let mut ledger_entries = BTreeMap::new();

    let error = resolve_request(
        &mut requests,
        &mut events,
        &mut accounts,
        &mut ledger_entries,
        ResolveRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "maintainer".to_string(),
            disposition: RequestDisposition::HiddenContext,
            event_id: "event_resolved".to_string(),
            settlement_event_id: "event_settled".to_string(),
            refund_ledger_entry_id: Some("ledger_refund".to_string()),
            reward_ledger_entry_id: Some("ledger_reward".to_string()),
            body: None,
            now_unix: 30,
        },
    )
    .unwrap_err();

    assert!(error.message.contains("exceeds i32 range"));
    assert_eq!(
        requests.get("req_1").unwrap().state,
        RequestState::Submitted
    );
    assert_eq!(accounts.get("user_public").unwrap().balance_credits, 0);
    assert!(events.is_empty());
    assert!(ledger_entries.is_empty());
}

#[test]
fn abandonment_requires_contributor_turn() {
    let mut requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    let mut events = BTreeMap::new();
    let mut accounts = BTreeMap::from([(
        "user_public".to_string(),
        UserCreditAccount {
            user_id: "user_public".to_string(),
            balance_credits: 0,
        },
    )]);

    let error = resolve_request(
        &mut requests,
        &mut events,
        &mut accounts,
        &mut BTreeMap::new(),
        ResolveRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "maintainer".to_string(),
            disposition: RequestDisposition::Abandoned,
            event_id: "event_resolved".to_string(),
            settlement_event_id: "event_settled".to_string(),
            refund_ledger_entry_id: None,
            reward_ledger_entry_id: None,
            body: None,
            now_unix: 30,
        },
    )
    .unwrap_err();

    assert!(error.message.contains("waiting on the contributor"));
}

#[test]
fn settlement_cannot_run_twice() {
    let mut request = submitted_request();
    request.state = RequestState::Resolved;
    request.settlement = Some(settlement_for(10, RequestDisposition::LowQuality, 20));
    let mut requests = BTreeMap::from([("req_1".to_string(), request)]);

    let error = resolve_request(
        &mut requests,
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        ResolveRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "maintainer".to_string(),
            disposition: RequestDisposition::Accepted,
            event_id: "event_resolved".to_string(),
            settlement_event_id: "event_settled".to_string(),
            refund_ledger_entry_id: Some("ledger_refund".to_string()),
            reward_ledger_entry_id: Some("ledger_reward".to_string()),
            body: None,
            now_unix: 30,
        },
    )
    .unwrap_err();

    assert!(error.message.contains("already closed"));
}

fn public_submit_input() -> SubmitRequestInput {
    SubmitRequestInput {
        id: "req_1".to_string(),
        repo_id: "owner/repo".to_string(),
        author_user_id: "user_public".to_string(),
        author_role: RequestActorRole::Public,
        base_audience: RequestBaseAudience::Public,
        target_branch: "main".to_string(),
        request_ref: "refs/scope/requests/req_1".to_string(),
        base_main_oid: "base".to_string(),
        head_oid: "head".to_string(),
        title: "Fix parser crash".to_string(),
        stake_credits: 10,
        stake_ledger_entry_id: Some("ledger_stake".to_string()),
        event_id: "event_created".to_string(),
        now_unix: 10,
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

fn submitted_request() -> Request {
    Request {
        id: "req_1".to_string(),
        repo_id: "owner/repo".to_string(),
        author_user_id: "user_public".to_string(),
        author_role: RequestActorRole::Public,
        base_audience: RequestBaseAudience::Public,
        target_branch: "main".to_string(),
        request_ref: "refs/scope/requests/req_1".to_string(),
        base_main_oid: "base".to_string(),
        head_oid: "head".to_string(),
        git_snapshot: None,
        title: "Fix parser crash".to_string(),
        state: RequestState::Submitted,
        stake_credits: 10,
        disposition: None,
        settlement: None,
        created_at_unix: 10,
        updated_at_unix: 10,
        resolved_at_unix: None,
    }
}
