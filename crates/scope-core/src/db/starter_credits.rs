//! Deterministic verified-account starter credit creation.

use super::{
    MetadataStore, acquire_aggregate_lock,
    request_rows::{
        credit_account_by_user_id, credit_ledger_entry_by_id, insert_credit_ledger_entry_row,
        save_credit_account_row,
    },
};
use crate::{
    domain::requests::{
        CreditLedgerEntryKind, GrantUserCreditsInput, PUBLIC_ACCOUNT_STARTER_CREDITS,
        UserCreditAccount, grant_user_credits,
    },
    error::ApiError,
};
use std::collections::BTreeMap;

pub(super) async fn ensure_verified_account_starter_credits<C>(
    conn: &C,
    user_id: &str,
    now_unix: u64,
) -> Result<UserCreditAccount, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    acquire_aggregate_lock(conn, "user-credit", user_id).await?;
    let ledger_id = starter_ledger_id(user_id);
    let account = credit_account_by_user_id(conn, user_id).await?;
    let entry = credit_ledger_entry_by_id(conn, &ledger_id).await?;
    match (account, entry) {
        (Some(account), Some(entry))
            if entry.user_id == user_id
                && entry.request_id.is_none()
                && entry.kind == CreditLedgerEntryKind::StarterGrant
                && entry.amount_credits
                    == i32::try_from(PUBLIC_ACCOUNT_STARTER_CREDITS)
                        .map_err(ApiError::internal)? =>
        {
            Ok(account)
        }
        (Some(_), Some(_)) => Err(ApiError::conflict(
            "verified account starter credit facts are inconsistent",
        )),
        (None, None) => {
            let mut accounts = BTreeMap::new();
            let mut entries = BTreeMap::new();
            let mutation = grant_user_credits(
                &mut accounts,
                &mut entries,
                GrantUserCreditsInput {
                    ledger_entry_id: ledger_id,
                    user_id: user_id.to_string(),
                    amount_credits: PUBLIC_ACCOUNT_STARTER_CREDITS,
                    now_unix,
                },
            )?;
            save_credit_account_row(conn, &mutation.account).await?;
            insert_credit_ledger_entry_row(conn, &mutation.ledger_entry).await?;
            Ok(mutation.account)
        }
        _ => Err(ApiError::conflict(
            "verified account starter credit facts are incomplete",
        )),
    }
}

impl MetadataStore {
    pub async fn credit_account_by_user_id(
        &self,
        user_id: &str,
    ) -> Result<Option<UserCreditAccount>, ApiError> {
        credit_account_by_user_id(self.db.as_ref(), user_id).await
    }
}

fn starter_ledger_id(user_id: &str) -> String {
    format!("starter-grant:{user_id}")
}
