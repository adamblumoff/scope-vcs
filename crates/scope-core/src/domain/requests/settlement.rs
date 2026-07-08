use super::{
    CreditLedgerEntry, CreditLedgerEntryKind, Request, RequestDisposition, RequestSettlement,
    UserCreditAccount, ensure_new_ledger_entry_id,
};
use crate::error::ApiError;
use std::collections::{BTreeMap, BTreeSet};

pub fn settlement_for(
    stake_credits: u32,
    disposition: RequestDisposition,
    settled_at_unix: u64,
) -> RequestSettlement {
    // `Accepted` is reserved for the merge flow; direct maintainer resolution
    // rejects it before settlement so acceptance cannot pay without a clean merge.
    let (refunded_credits, reward_credits) = match disposition {
        RequestDisposition::Accepted => (stake_credits, maximum_request_reward(stake_credits)),
        RequestDisposition::UsefulNotMerged => (stake_credits, stake_credits / 5),
        RequestDisposition::HiddenContext | RequestDisposition::NotAligned => (stake_credits, 0),
        RequestDisposition::Duplicate => (stake_credits / 2, 0),
        RequestDisposition::Abandoned | RequestDisposition::LowQuality => (0, 0),
    };
    RequestSettlement {
        disposition,
        stake_credits,
        refunded_credits,
        reward_credits,
        burned_credits: stake_credits.saturating_sub(refunded_credits),
        settled_at_unix,
    }
}

pub(super) fn maximum_request_reward(stake_credits: u32) -> u32 {
    stake_credits / 2
}

pub(super) fn settlement_event_body(settlement: &RequestSettlement) -> String {
    format!(
        "refunded={} reward={} burned={}",
        settlement.refunded_credits, settlement.reward_credits, settlement.burned_credits
    )
}

pub(super) struct CreditSettlementIds {
    pub(super) refund_ledger_entry_id: Option<String>,
    pub(super) reward_ledger_entry_id: Option<String>,
}

pub(super) struct CreditSettlementMutation {
    pub(super) account: Option<UserCreditAccount>,
    pub(super) ledger_entries: Vec<CreditLedgerEntry>,
}

pub(super) fn settle_request_credits(
    accounts: &mut BTreeMap<String, UserCreditAccount>,
    ledger_entries: &mut BTreeMap<String, CreditLedgerEntry>,
    request: &Request,
    settlement: &RequestSettlement,
    ids: CreditSettlementIds,
    now_unix: u64,
) -> Result<CreditSettlementMutation, ApiError> {
    let mut new_ledger_entries = Vec::new();
    let account = if request.stake_credits == 0 {
        None
    } else {
        let account = accounts.get(&request.author_user_id).ok_or_else(|| {
            ApiError::internal_message("request author credit account is missing")
        })?;
        let mut balance_credits = account.balance_credits;
        let mut reserved_ledger_entry_ids = BTreeSet::new();
        if settlement.refunded_credits > 0 {
            let entry_id = ids
                .refund_ledger_entry_id
                .clone()
                .ok_or_else(|| ApiError::bad_request("refund ledger entry id is required"))?;
            ensure_new_ledger_entry_id(ledger_entries, &mut reserved_ledger_entry_ids, &entry_id)?;
            let amount_credits = u32_to_i32(settlement.refunded_credits)?;
            balance_credits = balance_credits
                .checked_add(settlement.refunded_credits)
                .ok_or_else(|| ApiError::bad_request("credit balance overflow"))?;
            u32_to_i32(balance_credits)?;
            new_ledger_entries.push(CreditLedgerEntry {
                id: entry_id,
                user_id: request.author_user_id.clone(),
                request_id: Some(request.id.clone()),
                kind: CreditLedgerEntryKind::StakeRefund,
                amount_credits,
                created_at_unix: now_unix,
            });
        }
        if settlement.reward_credits > 0 {
            let entry_id = ids
                .reward_ledger_entry_id
                .clone()
                .ok_or_else(|| ApiError::bad_request("reward ledger entry id is required"))?;
            ensure_new_ledger_entry_id(ledger_entries, &mut reserved_ledger_entry_ids, &entry_id)?;
            let amount_credits = u32_to_i32(settlement.reward_credits)?;
            balance_credits = balance_credits
                .checked_add(settlement.reward_credits)
                .ok_or_else(|| ApiError::bad_request("credit balance overflow"))?;
            u32_to_i32(balance_credits)?;
            new_ledger_entries.push(CreditLedgerEntry {
                id: entry_id,
                user_id: request.author_user_id.clone(),
                request_id: Some(request.id.clone()),
                kind: CreditLedgerEntryKind::RequestReward,
                amount_credits,
                created_at_unix: now_unix,
            });
        }
        Some(UserCreditAccount {
            user_id: account.user_id.clone(),
            balance_credits,
        })
    };
    if let Some(account) = &account {
        accounts.insert(account.user_id.clone(), account.clone());
    }
    for entry in &new_ledger_entries {
        ledger_entries.insert(entry.id.clone(), entry.clone());
    }

    Ok(CreditSettlementMutation {
        account,
        ledger_entries: new_ledger_entries,
    })
}

pub(super) fn u32_to_i32(value: u32) -> Result<i32, ApiError> {
    i32::try_from(value).map_err(|_| ApiError::bad_request("credit amount exceeds i32 range"))
}
