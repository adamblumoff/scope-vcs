use super::MetadataStore;
use crate::domain::{
    policy::Visibility,
    requests::{
        AssessRequestInput, CreditLedgerEntryKind, GrantUserCreditsInput, MarkRequestReadyInput,
        RecordRequestRevisionInput, RecordWorkingRequestUploadInput, RequestActorRole,
        RequestAssessmentOutcome, RequestAudience, RequestEventKind, RequestReviewExitReason,
        RequestState, ReturnRequestToWorkingInput, SetRequestHoldInput, StartRequestInput,
        UpdateRequestDescriptionInput,
    },
    store::{
        AppCatalog, DEFAULT_GIT_FILE_MODE, RepoPublicationState, RepositoryMember,
        RepositoryMemberPermissions, SourceBlob, StoredRepository, UserAccount, app_catalog,
    },
};
use std::sync::Arc;
use tokio::{sync::Barrier, task::JoinSet};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ready_cap_is_serialized_and_failed_admission_charges_nothing() {
    let store = postgres_store();
    grant_credits(&store, "user_public", 100).await;
    for index in 0..4 {
        start_uploaded_request(
            &store,
            &format!("req_{index}"),
            &format!("request-{index}"),
            "user_public",
            RequestActorRole::Public,
        )
        .await;
    }

    let barrier = Arc::new(Barrier::new(4));
    let mut tasks = JoinSet::new();
    for index in 0..4 {
        let store = store.clone();
        let barrier = Arc::clone(&barrier);
        tasks.spawn(async move {
            barrier.wait().await;
            store
                .mark_request_ready(ready_input(
                    &format!("req_{index}"),
                    "user_public",
                    Some(1),
                    index,
                ))
                .await
        });
    }
    let mut successes = 0;
    let mut failures = 0;
    while let Some(result) = tasks.join_next().await {
        match result.unwrap() {
            Ok(_) => successes += 1,
            Err(error) if error.message.contains("at most 3") => failures += 1,
            Err(error) => panic!("unexpected Ready result: {}", error.message),
        }
    }

    assert_eq!((successes, failures), (3, 1));
    assert_eq!(
        store
            .requests_by_repo_author("owner/repo", "user_public")
            .await
            .unwrap()
            .into_iter()
            .filter(|request| request.state == RequestState::ReadyForReview)
            .count(),
        3
    );
    assert_eq!(
        store
            .credit_account_for_tests("user_public")
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        97
    );
    let ledger = store.credit_ledger_entries_for_tests().await.unwrap();
    assert_eq!(
        ledger
            .iter()
            .filter(|entry| entry.kind == CreditLedgerEntryKind::ReviewStakeDebit)
            .count(),
        3
    );
}

#[tokio::test]
async fn repeat_cycles_hold_invalidation_and_assessment_are_atomic_and_auditable() {
    let store = postgres_store();
    grant_credits(&store, "user_public", 100).await;
    start_uploaded_request(
        &store,
        "req_cycle",
        "cycle",
        "user_public",
        RequestActorRole::Public,
    )
    .await;

    let ready = store
        .mark_request_ready(ready_input("req_cycle", "user_public", Some(10), 1))
        .await
        .unwrap();
    assert_eq!(ready.credit_account.unwrap().balance_credits, 90);
    assert_eq!(ready.request.ready_queue_version, Some(1));
    store
        .set_request_hold(SetRequestHoldInput {
            request_id: "req_cycle".to_string(),
            actor_user_id: "user_owner".to_string(),
            actor_is_maintainer: false,
            held: true,
            event_id: "event_hold".to_string(),
            now_unix: 20,
        })
        .await
        .unwrap();
    let blocked = store
        .return_request_to_working(ReturnRequestToWorkingInput {
            request_id: "req_cycle".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_is_author: false,
            actor_is_maintainer: false,
            actor_can_mutate: false,
            reason: RequestReviewExitReason::AuthorReturned,
            event_id: "event_author_working".to_string(),
            now_unix: 21,
        })
        .await
        .unwrap_err();
    assert!(blocked.message.contains("held"));

    let working = store
        .return_request_to_working(ReturnRequestToWorkingInput {
            request_id: "req_cycle".to_string(),
            actor_user_id: "user_owner".to_string(),
            actor_is_author: false,
            actor_is_maintainer: false,
            actor_can_mutate: false,
            reason: RequestReviewExitReason::ChangesRequested,
            event_id: "event_changes_requested".to_string(),
            now_unix: 22,
        })
        .await
        .unwrap();
    assert_eq!(working.request.state, RequestState::Working);
    assert_eq!(working.request.held_at_unix, None);
    assert_eq!(working.credit_account.unwrap().balance_credits, 100);
    assert_eq!(working.request.ready_queue_version, Some(1));

    let second_ready = store
        .mark_request_ready(ready_input("req_cycle", "user_public", Some(20), 2))
        .await
        .unwrap();
    assert_eq!(second_ready.request.ready_queue_version, Some(2));
    let assessed = store
        .assess_request(assessment_input(
            "req_cycle",
            RequestAssessmentOutcome::Accepted,
            30,
            "accepted",
        ))
        .await
        .unwrap();
    assert_eq!(assessed.request.state, RequestState::Completed);
    assert_eq!(
        assessed.request.assessment_outcome,
        Some(RequestAssessmentOutcome::Accepted)
    );
    assert_eq!(assessed.credit_account.unwrap().balance_credits, 120);
    assert!(
        store
            .assess_request(assessment_input(
                "req_cycle",
                RequestAssessmentOutcome::Neutral,
                31,
                "immutable"
            ))
            .await
            .unwrap_err()
            .message
            .contains("ready")
    );

    let events = store
        .request_events_by_request_id("req_cycle")
        .await
        .unwrap();
    let kinds = events.iter().map(|event| event.kind).collect::<Vec<_>>();
    assert!(kinds.contains(&RequestEventKind::Held));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == RequestEventKind::ReadyForReview)
            .count(),
        2
    );
    assert!(kinds.contains(&RequestEventKind::Settled));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_assessment_settles_once() {
    let store = postgres_store();
    grant_credits(&store, "user_public", 100).await;
    start_uploaded_request(
        &store,
        "req_assess",
        "assess",
        "user_public",
        RequestActorRole::Public,
    )
    .await;
    store
        .mark_request_ready(ready_input("req_assess", "user_public", Some(10), 3))
        .await
        .unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = JoinSet::new();
    for suffix in ["a", "b"] {
        let store = store.clone();
        let barrier = Arc::clone(&barrier);
        tasks.spawn(async move {
            barrier.wait().await;
            store
                .assess_request(assessment_input(
                    "req_assess",
                    RequestAssessmentOutcome::Accepted,
                    30,
                    suffix,
                ))
                .await
        });
    }
    let mut successes = 0;
    let mut failures = 0;
    while let Some(result) = tasks.join_next().await {
        match result.unwrap() {
            Ok(_) => successes += 1,
            Err(_) => failures += 1,
        }
    }
    assert_eq!((successes, failures), (1, 1));
    assert_eq!(
        store
            .credit_account_for_tests("user_public")
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        110
    );
    assert_eq!(
        store.credit_ledger_entries_for_tests().await.unwrap().len(),
        4
    );
}

#[tokio::test]
async fn maintainer_role_ready_requests_do_not_consume_the_public_cap() {
    let store = postgres_store();
    grant_credits(&store, "user_public", 100).await;
    store
        .mutate_repository_for_tests("owner/repo", |repo| {
            repo.members.push(RepositoryMember {
                repo_id: "owner/repo".to_string(),
                user_id: "user_public".to_string(),
                permissions: RepositoryMemberPermissions::default(),
                created_at_unix: 2,
                updated_at_unix: 2,
            });
        })
        .await
        .unwrap();
    for index in 0..3 {
        let request_id = format!("req_member_{index}");
        start_uploaded_request(
            &store,
            &request_id,
            &format!("member-{index}"),
            "user_public",
            RequestActorRole::Member,
        )
        .await;
        store
            .mark_request_ready(ready_input(&request_id, "user_public", None, index))
            .await
            .unwrap();
    }

    store
        .mutate_repository_for_tests("owner/repo", |repo| repo.members.clear())
        .await
        .unwrap();

    start_uploaded_request(
        &store,
        "req_public_after_member",
        "public-after-member",
        "user_public",
        RequestActorRole::Public,
    )
    .await;
    let ready = store
        .mark_request_ready(ready_input(
            "req_public_after_member",
            "user_public",
            Some(1),
            4,
        ))
        .await
        .unwrap();
    assert_eq!(ready.request.state, RequestState::ReadyForReview);
    assert_eq!(ready.credit_account.unwrap().balance_credits, 99);
}

#[tokio::test]
async fn membership_changes_do_not_rewrite_request_credit_contract() {
    let store = postgres_store();
    grant_credits(&store, "user_public", 100).await;
    start_uploaded_request(
        &store,
        "req_public_promoted",
        "public-promoted",
        "user_public",
        RequestActorRole::Public,
    )
    .await;
    store
        .mutate_repository_for_tests("owner/repo", |repo| {
            repo.members.push(RepositoryMember {
                repo_id: "owner/repo".to_string(),
                user_id: "user_public".to_string(),
                permissions: RepositoryMemberPermissions::default(),
                created_at_unix: 4,
                updated_at_unix: 4,
            });
        })
        .await
        .unwrap();
    let promoted = store
        .mark_request_ready(ready_input(
            "req_public_promoted",
            "user_public",
            Some(5),
            50,
        ))
        .await
        .unwrap();
    assert_eq!(promoted.request.current_stake_credits, 5);
    assert_eq!(promoted.credit_account.unwrap().balance_credits, 95);

    start_uploaded_request(
        &store,
        "req_member_removed",
        "member-removed",
        "user_public",
        RequestActorRole::Member,
    )
    .await;
    store
        .mutate_repository_for_tests("owner/repo", |repo| repo.members.clear())
        .await
        .unwrap();
    let removed = store
        .mark_request_ready(ready_input("req_member_removed", "user_public", None, 51))
        .await
        .unwrap();
    assert_eq!(removed.request.current_stake_credits, 0);
    assert!(removed.credit_account.is_none());
    assert_eq!(
        store
            .credit_account_for_tests("user_public")
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        95
    );
    assert_eq!(
        store
            .credit_ledger_entries_for_tests()
            .await
            .unwrap()
            .iter()
            .filter(|entry| entry.kind == CreditLedgerEntryKind::ReviewStakeDebit)
            .count(),
        1
    );
}

#[tokio::test]
async fn membership_revocation_blocks_private_author_lifecycle_commands() {
    let store = postgres_store();
    store
        .mutate_repository_for_tests("owner/repo", |repo| {
            repo.members.push(RepositoryMember {
                repo_id: "owner/repo".to_string(),
                user_id: "user_public".to_string(),
                permissions: RepositoryMemberPermissions::default(),
                created_at_unix: 2,
                updated_at_unix: 2,
            });
        })
        .await
        .unwrap();
    for (request_id, name) in [
        ("req_private_ready", "private-ready"),
        ("req_private_return", "private-return"),
        ("req_private_edit", "private-edit"),
        ("req_private_revision", "private-revision"),
    ] {
        start_uploaded_request_with_audience(
            &store,
            request_id,
            name,
            "user_public",
            RequestActorRole::Member,
            RequestAudience::Private,
        )
        .await;
    }
    store
        .mark_request_ready(ready_input("req_private_return", "user_public", None, 60))
        .await
        .unwrap();
    store
        .mark_request_ready(ready_input("req_private_edit", "user_public", None, 61))
        .await
        .unwrap();
    store
        .mutate_repository_for_tests("owner/repo", |repo| repo.members.clear())
        .await
        .unwrap();

    let ready_error = store
        .mark_request_ready(ready_input("req_private_ready", "user_public", None, 61))
        .await
        .unwrap_err();
    assert!(ready_error.message.contains("mutation access"));
    let working_error = store
        .return_request_to_working(ReturnRequestToWorkingInput {
            request_id: "req_private_return".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_is_author: false,
            actor_is_maintainer: false,
            actor_can_mutate: false,
            reason: RequestReviewExitReason::AuthorReturned,
            event_id: "event_private_return_after_removal".to_string(),
            now_unix: 70,
        })
        .await
        .unwrap_err();
    assert!(working_error.message.contains("mutation access"));
    let edit_error = store
        .update_request_description_with_review_invalidation(UpdateRequestDescriptionInput {
            request_id: "req_private_edit".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_edit_description: true,
            event_id: "event_private_edit_after_removal".to_string(),
            description_markdown: "revoked edit".to_string(),
            now_unix: 71,
        })
        .await
        .unwrap_err();
    assert!(edit_error.message.contains("mutation access"));
    let revision_error = store
        .record_request_revision_with_review_invalidation(RecordRequestRevisionInput {
            request_id: "req_private_revision".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: Some("head-req_private_revision".to_string()),
            new_head_oid: "head-private-revoked".to_string(),
            git_snapshot: SourceBlob {
                object_key: "objects/private-revoked".to_string(),
                sha256: "sha256-private-revoked".to_string(),
                git_oid: "head-private-revoked".to_string(),
                git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                size_bytes: 1,
            },
            event_id: "event_private_revision_after_removal".to_string(),
            body: None,
            now_unix: 72,
        })
        .await
        .unwrap_err();
    assert!(revision_error.message.contains("branch edit access"));
    assert_eq!(
        store
            .request_for_tests("req_private_ready")
            .await
            .unwrap()
            .unwrap()
            .state,
        RequestState::Working
    );
    assert_eq!(
        store
            .request_for_tests("req_private_return")
            .await
            .unwrap()
            .unwrap()
            .state,
        RequestState::ReadyForReview
    );
    let private_edit = store
        .request_for_tests("req_private_edit")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(private_edit.state, RequestState::ReadyForReview);
    assert!(private_edit.description_markdown.is_empty());
    assert_eq!(
        store
            .request_for_tests("req_private_revision")
            .await
            .unwrap()
            .unwrap()
            .head_oid,
        "head-req_private_revision"
    );
}

#[tokio::test]
async fn insufficient_credits_roll_back_and_maintainer_ready_is_unlimited_and_unstaked() {
    let store = postgres_store();
    grant_credits(&store, "user_public", 5).await;
    start_uploaded_request(
        &store,
        "req_poor",
        "poor",
        "user_public",
        RequestActorRole::Public,
    )
    .await;
    let error = store
        .mark_request_ready(ready_input("req_poor", "user_public", Some(6), 4))
        .await
        .unwrap_err();
    assert!(error.message.contains("insufficient"));
    assert_eq!(
        store
            .request_for_tests("req_poor")
            .await
            .unwrap()
            .unwrap()
            .state,
        RequestState::Working
    );
    assert_eq!(
        store.credit_ledger_entries_for_tests().await.unwrap().len(),
        1
    );

    for index in 0..4 {
        let request_id = format!("req_owner_{index}");
        start_uploaded_request(
            &store,
            &request_id,
            &format!("owner-{index}"),
            "user_owner",
            RequestActorRole::Owner,
        )
        .await;
        store
            .mark_request_ready(ready_input(&request_id, "user_owner", None, 10 + index))
            .await
            .unwrap();
    }
    assert_eq!(
        store
            .requests_by_repo_author("owner/repo", "user_owner")
            .await
            .unwrap()
            .into_iter()
            .filter(|request| request.state == RequestState::ReadyForReview)
            .count(),
        4
    );
    assert!(
        store
            .credit_account_for_tests("user_owner")
            .await
            .unwrap()
            .is_none()
    );
}

fn postgres_store() -> MetadataStore {
    let store = MetadataStore::connect_fresh_for_tests(
        &super::TestDatabaseTarget::required().expect("test database target"),
    )
    .expect("connect test database");
    store.seed_catalog_for_tests(catalog_with_repo()).unwrap();
    store
}

fn catalog_with_repo() -> AppCatalog {
    let owner = user("user_owner", "owner");
    let public = user("user_public", "public");
    let mut repo = StoredRepository::new(&owner, "repo", Visibility::Public).unwrap();
    repo.record.publication_state = RepoPublicationState::Published;
    let mut catalog = app_catalog();
    catalog.users.insert(owner.id.clone(), owner);
    catalog.users.insert(public.id.clone(), public);
    catalog.repositories.insert(repo.record.id.clone(), repo);
    catalog
}

fn user(id: &str, handle: &str) -> UserAccount {
    UserAccount {
        id: id.to_string(),
        handle: handle.to_string(),
        email: format!("{handle}@example.com"),
        email_verified: true,
    }
}

async fn grant_credits(store: &MetadataStore, user_id: &str, amount: u32) {
    store
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: format!("starter:{user_id}"),
            user_id: user_id.to_string(),
            amount_credits: amount,
            now_unix: 1,
        })
        .await
        .unwrap();
}

async fn start_uploaded_request(
    store: &MetadataStore,
    request_id: &str,
    name: &str,
    author_user_id: &str,
    author_role: RequestActorRole,
) {
    start_uploaded_request_with_audience(
        store,
        request_id,
        name,
        author_user_id,
        author_role,
        RequestAudience::Public,
    )
    .await;
}

async fn start_uploaded_request_with_audience(
    store: &MetadataStore,
    request_id: &str,
    name: &str,
    author_user_id: &str,
    author_role: RequestActorRole,
    audience: RequestAudience,
) {
    store
        .start_request(StartRequestInput {
            id: request_id.to_string(),
            repo_id: "owner/repo".to_string(),
            name: name.to_string(),
            author_user_id: author_user_id.to_string(),
            title: None,
            author_role,
            audience,
            base_main_oid: "base".to_string(),
            event_id: format!("event_started_{request_id}"),
            now_unix: 2,
        })
        .await
        .unwrap();
    store
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: request_id.to_string(),
            actor_user_id: author_user_id.to_string(),
            actor_can_edit: false,
            expected_old_head_oid: None,
            new_head_oid: format!("head-{request_id}"),
            git_snapshot: SourceBlob {
                object_key: format!("objects/{request_id}"),
                sha256: format!("sha256-{request_id}"),
                git_oid: format!("head-{request_id}"),
                git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                size_bytes: 1,
            },
            now_unix: 3,
        })
        .await
        .unwrap();
}

fn ready_input(
    request_id: &str,
    actor_user_id: &str,
    stake_credits: Option<u32>,
    unique: usize,
) -> MarkRequestReadyInput {
    MarkRequestReadyInput {
        request_id: request_id.to_string(),
        actor_user_id: actor_user_id.to_string(),
        actor_is_author: false,
        actor_can_mutate: false,
        stake_credits,
        public_ready_count: usize::MAX,
        ready_queue_version: 0,
        event_id: format!("event_ready_{unique}"),
        stake_ledger_entry_id: stake_credits.map(|_| format!("ledger_stake_{unique}")),
        now_unix: 10 + unique as u64,
    }
}

fn assessment_input(
    request_id: &str,
    outcome: RequestAssessmentOutcome,
    now_unix: u64,
    suffix: &str,
) -> AssessRequestInput {
    AssessRequestInput {
        request_id: request_id.to_string(),
        actor_user_id: "user_owner".to_string(),
        actor_is_maintainer: false,
        outcome,
        body_markdown: (outcome == RequestAssessmentOutcome::Rejected)
            .then(|| "Concrete rejection reason".to_string()),
        assessed_event_id: format!("event_assessed_{suffix}"),
        settled_event_id: Some(format!("event_settled_{suffix}")),
        refund_ledger_entry_id: (outcome != RequestAssessmentOutcome::Rejected)
            .then(|| format!("ledger_refund_{suffix}")),
        reward_ledger_entry_id: (outcome == RequestAssessmentOutcome::Accepted)
            .then(|| format!("ledger_reward_{suffix}")),
        now_unix,
    }
}
