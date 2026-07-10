use super::*;
use crate::domain::{
    policy::Visibility,
    requests::{RequestActorRole, RequestBaseAudience, RequestDisposition, RequestState},
    store::{
        AppCatalog, DEFAULT_GIT_FILE_MODE, RepoPublicationState, SourceBlob, StoredRepository,
        UserAccount, app_catalog,
    },
};

#[tokio::test]
async fn request_submission_and_resolution_update_credit_facts() {
    let store = postgres_store();

    store
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "ledger_grant".to_string(),
            user_id: "user_public".to_string(),
            amount_credits: 20,
            now_unix: 1,
        })
        .await
        .unwrap();
    submit_public_request(&store).await;
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
    store
        .read(|catalog| {
            assert_eq!(
                catalog
                    .user_credit_accounts
                    .get("user_public")
                    .unwrap()
                    .balance_credits,
                22
            );
            assert_eq!(
                catalog.requests.get("req_1").unwrap().resolved_at_unix,
                Some(3)
            );
            assert_eq!(catalog.request_events.len(), 3);
            assert_eq!(catalog.credit_ledger_entries.len(), 4);
            Ok(())
        })
        .await
        .unwrap();
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
    store
        .read(|catalog| {
            assert_eq!(
                catalog.requests.get("req_1").unwrap().state,
                RequestState::Working
            );
            assert!(catalog.request_events.is_empty());
            assert!(catalog.credit_ledger_entries.is_empty());
            Ok(())
        })
        .await
        .unwrap();
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
async fn non_maintainer_cannot_resolve_request() {
    let store = postgres_store();
    store
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "ledger_grant".to_string(),
            user_id: "user_public".to_string(),
            amount_credits: 20,
            now_unix: 1,
        })
        .await
        .unwrap();
    submit_public_request(&store).await;

    let error = store
        .resolve_request(ResolveRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            disposition: RequestDisposition::Accepted,
            event_id: "event_resolved".to_string(),
            settlement_event_id: "event_settled".to_string(),
            refund_ledger_entry_id: Some("ledger_refund".to_string()),
            reward_ledger_entry_id: Some("ledger_reward".to_string()),
            body: None,
            now_unix: 3,
        })
        .await
        .unwrap_err();

    assert!(error.message.contains("repo maintainer required"));
    store
        .read(|catalog| {
            assert_eq!(
                catalog.requests.get("req_1").unwrap().resolved_at_unix,
                None
            );
            assert_eq!(catalog.request_events.len(), 1);
            assert_eq!(
                catalog
                    .user_credit_accounts
                    .get("user_public")
                    .unwrap()
                    .balance_credits,
                10
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn request_facts_round_trip() {
    let store = postgres_store();

    store
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "ledger_grant".to_string(),
            user_id: "user_public".to_string(),
            amount_credits: 20,
            now_unix: 1,
        })
        .await
        .unwrap();
    submit_public_request(&store).await;
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
    let mut invalid_ref = public_start_input();
    invalid_ref.id = "req_2".to_string();
    let error = store.start_request(invalid_ref).await.unwrap_err();
    assert!(error.message.contains("request ref must match"));

    store
        .read(|catalog| {
            assert_eq!(catalog.requests.get("req_1").unwrap().head_oid, "head_2");
            assert_eq!(
                catalog
                    .user_credit_accounts
                    .get("user_public")
                    .unwrap()
                    .balance_credits,
                10
            );
            assert_eq!(catalog.request_events.len(), 2);
            Ok(())
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
