use super::{MetadataStore, TestDatabaseTarget};
use crate::{
    auth::clerk::ClerkIdentity,
    domain::requests::{CreditLedgerEntryKind, PUBLIC_ACCOUNT_STARTER_CREDITS},
};
use std::sync::Arc;
use tokio::{sync::Barrier, task::JoinSet};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn verified_account_gets_exactly_one_deterministic_starter_grant() {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let identity = ClerkIdentity {
        user_id: "verified-subject".to_string(),
        email: Some("verified@example.com".to_string()),
        email_verified: true,
    };
    let barrier = Arc::new(Barrier::new(6));
    let mut tasks = JoinSet::new();
    for _ in 0..6 {
        let store = store.clone();
        let identity = identity.clone();
        let barrier = Arc::clone(&barrier);
        tasks.spawn(async move {
            barrier.wait().await;
            store.resolve_clerk_user(&identity).await
        });
    }
    let mut user_id = None;
    while let Some(result) = tasks.join_next().await {
        let user = result.unwrap().unwrap();
        assert!(user_id.as_ref().is_none_or(|id| id == &user.id));
        user_id = Some(user.id);
    }
    let user_id = user_id.unwrap();
    assert_eq!(
        store
            .credit_account_for_tests(&user_id)
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        PUBLIC_ACCOUNT_STARTER_CREDITS
    );
    let ledger = store.credit_ledger_entries_for_tests().await.unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].kind, CreditLedgerEntryKind::StarterGrant);
    assert_eq!(
        ledger[0].amount_credits,
        i32::try_from(PUBLIC_ACCOUNT_STARTER_CREDITS).unwrap()
    );
    assert_eq!(ledger[0].request_id, None);
}
