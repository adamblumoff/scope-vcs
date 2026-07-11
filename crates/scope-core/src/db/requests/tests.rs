use super::*;
use crate::domain::{
    policy::Visibility,
    requests::{
        CreditLedgerEntryKind, RequestActorRole, RequestBaseAudience, RequestDisposition,
        RequestState,
    },
    store::{
        AppCatalog, DEFAULT_GIT_FILE_MODE, RepoPublicationState, SourceBlob, StoredRepository,
        UserAccount, app_catalog,
    },
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

#[tokio::test]
async fn request_submission_and_resolution_update_credit_facts() {
    let store = postgres_store();

    grant_public_credits(&store).await;
    submit_public_request(&store).await;
    let unauthorized = store
        .resolve_request(ResolveRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            disposition: RequestDisposition::Accepted,
            event_id: "rejected".to_string(),
            settlement_event_id: "rejected-settlement".to_string(),
            refund_ledger_entry_id: None,
            reward_ledger_entry_id: None,
            body: None,
            now_unix: 2,
        })
        .await
        .unwrap_err();
    assert!(unauthorized.message.contains("repo maintainer required"));
    store
        .record_request_revision(RecordRequestRevisionInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: Some("head".to_string()),
            new_head_oid: "head_2".to_string(),
            git_snapshot: None,
            event_id: "event_revision".to_string(),
            body: None,
            now_unix: 2,
        })
        .await
        .unwrap();
    let mutation = store
        .resolve_request(ResolveRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_owner".to_string(),
            disposition: RequestDisposition::UsefulNotMerged,
            event_id: "event_resolved".to_string(),
            settlement_event_id: "event_settled".to_string(),
            refund_ledger_entry_id: Some("ledger_refund".to_string()),
            reward_ledger_entry_id: Some("ledger_reward".to_string()),
            body: None,
            now_unix: 3,
        })
        .await
        .unwrap();

    assert_eq!(mutation.request.settlement.unwrap().reward_credits, 2);
    assert_eq!(
        store
            .credit_account_for_tests("user_public")
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        22
    );
    let request = store.request_for_tests("req_1").await.unwrap().unwrap();
    assert_eq!(request.resolved_at_unix, Some(3));
    assert_eq!(request.head_oid, "head_2");
    assert_eq!(store.request_events_for_tests().await.unwrap().len(), 4);
    assert_eq!(
        store.credit_ledger_entries_for_tests().await.unwrap().len(),
        4
    );
}

#[tokio::test]
async fn public_user_cannot_choose_owner_role_to_skip_stake() {
    let store = postgres_store();
    store.start_request(public_start_input()).await.unwrap();
    store
        .record_working_request_upload(public_upload_input())
        .await
        .unwrap();
    let mut input = public_submit_input();
    input.stake_credits = 0;
    input.stake_ledger_entry_id = None;

    let error = store.submit_request(input).await.unwrap_err();

    assert!(
        error
            .message
            .contains("public requests require credit stake")
    );
    assert_eq!(
        store
            .request_for_tests("req_1")
            .await
            .unwrap()
            .unwrap()
            .state,
        RequestState::Working
    );
    assert!(store.request_events_for_tests().await.unwrap().is_empty());
    assert!(
        store
            .credit_ledger_entries_for_tests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn owner_submission_derives_private_base_without_credits() {
    let store = postgres_store();
    let mut input = public_start_input();
    input.id = "req_owner".to_string();
    input.request_ref = "refs/scope/requests/req_owner".to_string();
    input.author_user_id = "user_owner".to_string();
    input.author_role = RequestActorRole::Public;
    input.base_audience = RequestBaseAudience::Public;
    let start = store.start_request(input).await.unwrap();
    store
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: start.request.id.clone(),
            actor_user_id: "user_owner".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: "head".to_string(),
            git_snapshot: source_blob("head"),
            now_unix: 2,
        })
        .await
        .unwrap();
    let mutation = store
        .submit_request(SubmitRequestInput {
            request_id: start.request.id,
            actor_user_id: "user_owner".to_string(),
            expected_head_oid: "head".to_string(),
            stake_credits: 0,
            stake_ledger_entry_id: None,
            event_id: "event_owner".to_string(),
            now_unix: 3,
        })
        .await
        .unwrap();

    assert_eq!(mutation.request.author_role, RequestActorRole::Owner);
    assert_eq!(mutation.request.base_audience, RequestBaseAudience::Private);
    assert!(mutation.account.is_none());
    assert!(mutation.ledger_entry.is_none());
}

#[tokio::test]
async fn repo_delete_waits_for_resolution_and_does_not_refund_settled_stake_twice() {
    assert_delete_serializes_with_credit_mutation(CreditMutation::Resolve, 22).await;
}

#[tokio::test]
async fn repo_delete_waits_for_submission_and_refunds_the_committed_stake() {
    assert_delete_serializes_with_credit_mutation(CreditMutation::Submit, 20).await;
}

enum CreditMutation {
    Submit,
    Resolve,
}

async fn assert_delete_serializes_with_credit_mutation(
    mutation: CreditMutation,
    expected_balance: u32,
) {
    let store = postgres_store();
    grant_public_credits(&store).await;
    match mutation {
        CreditMutation::Submit => {
            store.start_request(public_start_input()).await.unwrap();
            store
                .record_working_request_upload(public_upload_input())
                .await
                .unwrap();
        }
        CreditMutation::Resolve => submit_public_request(&store).await,
    }

    let credit_guard = store.db.begin().await.unwrap();
    acquire_aggregate_lock(&credit_guard, "user-credit", "user_public")
        .await
        .unwrap();

    let mutation_store = store.clone();
    let mutation = tokio::spawn(async move {
        match mutation {
            CreditMutation::Submit => mutation_store
                .submit_request(public_submit_input())
                .await
                .map(|_| ()),
            CreditMutation::Resolve => mutation_store
                .resolve_request(ResolveRequestInput {
                    request_id: "req_1".to_string(),
                    actor_user_id: "user_owner".to_string(),
                    disposition: RequestDisposition::UsefulNotMerged,
                    event_id: "event_resolved".to_string(),
                    settlement_event_id: "event_settled".to_string(),
                    refund_ledger_entry_id: Some("ledger_refund".to_string()),
                    reward_ledger_entry_id: Some("ledger_reward".to_string()),
                    body: None,
                    now_unix: 5,
                })
                .await
                .map(|_| ()),
        }
    });
    wait_until_aggregate_lock_is_held(&store, "repository", "owner/repo").await;
    let delete_store = store.clone();
    let delete = tokio::spawn(async move {
        delete_store
            .delete_repo("owner", "repo", "user_owner")
            .await
    });
    wait_until_aggregate_lock_is_waited_on(&store, "repository").await;

    assert!(
        !mutation.is_finished(),
        "credit mutation should wait for credit lock"
    );
    assert!(
        !delete.is_finished(),
        "deletion should wait behind the repository lock"
    );
    credit_guard.commit().await.unwrap();

    mutation.await.unwrap().unwrap();
    delete.await.unwrap().unwrap();
    assert!(
        store
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .credit_account_for_tests("user_public")
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        expected_balance
    );
    assert_eq!(
        store
            .credit_ledger_entries_for_tests()
            .await
            .unwrap()
            .into_iter()
            .filter(|entry| entry.kind == CreditLedgerEntryKind::StakeRefund)
            .count(),
        1
    );
}

async fn grant_public_credits(store: &MetadataStore) {
    store
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "ledger_grant".to_string(),
            user_id: "user_public".to_string(),
            amount_credits: 20,
            now_unix: 1,
        })
        .await
        .unwrap();
}

fn postgres_store() -> MetadataStore {
    let target = super::super::TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    store.seed_catalog_for_tests(catalog_with_repo()).unwrap();
    store
}

async fn wait_until_aggregate_lock_is_held(store: &MetadataStore, namespace: &str, id: &str) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let probe = store.db.begin().await.unwrap();
        probe
            .execute(Statement::from_string(
                DatabaseBackend::Postgres,
                "SET LOCAL lock_timeout = '10ms'",
            ))
            .await
            .unwrap();
        match acquire_aggregate_lock(&probe, namespace, id).await {
            Ok(()) => {
                probe.rollback().await.unwrap();
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "timed out waiting for {namespace}:{id} to be locked"
                );
                tokio::task::yield_now().await;
            }
            Err(error) if error.message.contains("lock timeout") => {
                probe.rollback().await.unwrap();
                return;
            }
            Err(error) => {
                let _ = probe.rollback().await;
                panic!("failed to probe {namespace}:{id} lock: {}", error.message);
            }
        }
    }
}

async fn wait_until_aggregate_lock_is_waited_on(store: &MetadataStore, namespace: &str) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let waiting = store
            .db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT EXISTS (\
                    SELECT 1 FROM pg_stat_activity AS activity \
                    JOIN pg_locks AS relation_lock ON relation_lock.pid = activity.pid \
                    WHERE activity.application_name = $1 \
                      AND activity.wait_event_type = 'Lock' \
                      AND relation_lock.relation = to_regclass('scope_metadata_locks')::oid\
                ) AS waiting",
                [format!("scope-test-lock:{namespace}").into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<bool>("", "waiting")
            .unwrap();
        if waiting {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for a blocked {namespace} aggregate lock"
        );
        tokio::task::yield_now().await;
    }
}

fn catalog_with_repo() -> AppCatalog {
    let owner = UserAccount {
        id: "user_owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.com".to_string(),
        email_verified: true,
    };
    let public_user = UserAccount {
        id: "user_public".to_string(),
        handle: "public".to_string(),
        email: "public@example.com".to_string(),
        email_verified: true,
    };
    let mut repo = StoredRepository::new(&owner, "repo", Visibility::Public).unwrap();
    repo.record.publication_state = RepoPublicationState::Published;

    let mut catalog = app_catalog();
    catalog.users.insert(owner.id.clone(), owner);
    catalog.users.insert(public_user.id.clone(), public_user);
    catalog.repositories.insert(repo.record.id.clone(), repo);
    catalog
}

async fn submit_public_request(store: &MetadataStore) {
    store.start_request(public_start_input()).await.unwrap();
    store
        .record_working_request_upload(public_upload_input())
        .await
        .unwrap();
    store.submit_request(public_submit_input()).await.unwrap();
}

fn public_start_input() -> StartRequestInput {
    StartRequestInput {
        id: "req_1".to_string(),
        repo_id: "owner/repo".to_string(),
        author_user_id: "user_public".to_string(),
        title: "Fix parser crash".to_string(),
        author_role: RequestActorRole::Public,
        base_audience: RequestBaseAudience::Public,
        target_branch: "main".to_string(),
        request_ref: "refs/scope/requests/req_1".to_string(),
        base_main_oid: "base".to_string(),
        now_unix: 2,
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
        now_unix: 3,
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
        now_unix: 4,
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
