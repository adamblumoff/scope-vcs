use super::{RequestAssessmentOutcome, validate_required_id};
use crate::error::ApiError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PUBLIC_ACCOUNT_STARTER_CREDITS: u32 = 100;

const REPO_DELETE_REFUND_LEDGER_ENTRY_PREFIX: &str = "repo_delete_refund:";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct RequestSettlement {
    pub outcome: RequestAssessmentOutcome,
    pub stake_credits: u32,
    pub refunded_credits: u32,
    pub reward_credits: u32,
    pub burned_credits: u32,
    pub settled_at_unix: u64,
}

pub fn settlement_for(
    stake_credits: u32,
    outcome: RequestAssessmentOutcome,
    settled_at_unix: u64,
) -> RequestSettlement {
    let (refunded_credits, reward_credits) = match outcome {
        RequestAssessmentOutcome::Accepted => (stake_credits, stake_credits),
        RequestAssessmentOutcome::Neutral => (stake_credits, 0),
        RequestAssessmentOutcome::Rejected => (0, 0),
    };
    RequestSettlement {
        outcome,
        stake_credits,
        refunded_credits,
        reward_credits,
        burned_credits: stake_credits.saturating_sub(refunded_credits),
        settled_at_unix,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserCreditAccount {
    pub user_id: String,
    pub balance_credits: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum CreditLedgerEntryKind {
    StarterGrant,
    ReviewStakeDebit,
    ReviewStakeRefund,
    AssessmentReward,
    AdminAdjustment,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditLedgerEntry {
    pub id: String,
    pub user_id: String,
    pub request_id: Option<String>,
    pub kind: CreditLedgerEntryKind,
    pub amount_credits: i32,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug)]
pub struct GrantUserCreditsInput {
    pub ledger_entry_id: String,
    pub user_id: String,
    pub amount_credits: u32,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreditAccountMutation {
    pub account: UserCreditAccount,
    pub ledger_entry: CreditLedgerEntry,
}

pub fn grant_user_credits(
    accounts: &mut BTreeMap<String, UserCreditAccount>,
    ledger_entries: &mut BTreeMap<String, CreditLedgerEntry>,
    input: GrantUserCreditsInput,
) -> Result<CreditAccountMutation, ApiError> {
    validate_required_id("ledger entry id", &input.ledger_entry_id)?;
    validate_required_id("user id", &input.user_id)?;
    if input.amount_credits == 0 {
        return Err(ApiError::bad_request(
            "credit grant amount must be positive",
        ));
    }
    ensure_ledger_entry_id_available(ledger_entries, &input.ledger_entry_id)?;
    let current_balance = accounts
        .get(&input.user_id)
        .map(|account| account.balance_credits)
        .unwrap_or(0);
    let balance_credits = current_balance
        .checked_add(input.amount_credits)
        .ok_or_else(|| ApiError::bad_request("credit balance overflow"))?;
    u32_to_i32(balance_credits)?;
    let amount_credits = u32_to_i32(input.amount_credits)?;

    let account = UserCreditAccount {
        user_id: input.user_id.clone(),
        balance_credits,
    };
    let ledger_entry = CreditLedgerEntry {
        id: input.ledger_entry_id,
        user_id: input.user_id,
        request_id: None,
        kind: CreditLedgerEntryKind::StarterGrant,
        amount_credits,
        created_at_unix: input.now_unix,
    };
    accounts.insert(account.user_id.clone(), account.clone());
    ledger_entries.insert(ledger_entry.id.clone(), ledger_entry.clone());
    Ok(CreditAccountMutation {
        account,
        ledger_entry,
    })
}

fn ensure_ledger_entry_id_available(
    ledger_entries: &BTreeMap<String, CreditLedgerEntry>,
    ledger_entry_id: &str,
) -> Result<(), ApiError> {
    validate_required_id("credit ledger entry id", ledger_entry_id)?;
    if ledger_entry_id.starts_with(REPO_DELETE_REFUND_LEDGER_ENTRY_PREFIX) {
        return Err(ApiError::bad_request(
            "credit ledger entry id uses a working internal prefix",
        ));
    }
    if ledger_entries.contains_key(ledger_entry_id) {
        Err(ApiError::conflict("credit ledger entry already exists"))
    } else {
        Ok(())
    }
}

fn u32_to_i32(value: u32) -> Result<i32, ApiError> {
    i32::try_from(value).map_err(|_| ApiError::bad_request("credit amount exceeds i32 range"))
}
