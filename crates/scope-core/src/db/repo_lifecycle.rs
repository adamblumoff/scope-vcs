use super::{
    MetadataStore, acquire_metadata_write_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    cleanup_queue::{complete_pending_repo_storage_cleanup, pending_repo_storage_cleanup_exists},
    entities,
    repo_effects::save_repo_effects,
    repository_from_model,
    repository_rows::insert_repository,
    request_rows::{
        credit_account_by_user_id, credit_ledger_entry_by_id, insert_credit_ledger_entry_row,
        request_stake_debit_entry_for_request_id, requests_by_repo_id, save_credit_account_row,
    },
};
use crate::domain::{
    policy::Visibility,
    repo_actions::{create_repo as create_repo_command, delete_repo as delete_repo_command},
    requests::{CreditLedgerEntry, CreditLedgerEntryKind, Request, UserCreditAccount},
    store::{FirstPushToken, GitPushToken, SourceBlob, StoredRepository, repo_id},
};
use crate::error::ApiError;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use std::sync::Arc;

impl MetadataStore {
    pub async fn create_repo_with_init_tokens<F>(
        &self,
        owner_user_id: &str,
        name: &str,
        default_visibility: Visibility,
        first_push_token: FirstPushToken,
        git_push_token: GitPushToken,
        cleanup_pending_storage: F,
    ) -> Result<StoredRepository, ApiError>
    where
        F: FnOnce(&str, &str) -> Result<(), ApiError> + Send + 'static,
    {
        let owner_user_id = owner_user_id.to_string();
        let name = name.to_string();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_metadata_write_lock(&tx).await?;
        let owner = entities::user::Entity::find_by_id(owner_user_id)
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::internal_message("signed-in user was not persisted"))?
            .try_into_domain()?;
        let mutation = create_repo_command(
            &owner,
            &name,
            default_visibility,
            first_push_token,
            git_push_token,
        )?;
        let repo = mutation.result;
        if entities::repository::Entity::find_by_id(repo.record.id.clone())
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .is_some()
        {
            return Err(ApiError::conflict(format!(
                "repo {} already exists",
                repo.record.id
            )));
        }

        if pending_repo_storage_cleanup_exists(&tx, &repo.record.id).await? {
            cleanup_pending_storage(&repo.record.owner_handle, &repo.record.name)?;
            complete_pending_repo_storage_cleanup(&tx, &repo.record.id).await?;
        }

        insert_repository(&tx, &repo).await?;
        save_repo_effects(&tx, &mutation.effects).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(repo)
    }

    pub async fn delete_repo(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
    ) -> Result<String, ApiError> {
        let repo_id = repo_id(owner, name);
        let owner = owner.to_string();
        let name = name.to_string();
        let user_id = user_id.to_string();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_metadata_write_lock(&tx).await?;
        let repo = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| crate::domain::repo_actions::hidden_repo_not_found(&owner, &name))?;
        let repo = repository_from_model(&tx, repo).await?;
        let mutation = delete_repo_command(&repo, &user_id, &owner, &name)?;
        refund_open_request_stakes_for_repo_postgres(&tx, &repo_id, unix_now()).await?;
        let request_git_snapshots = request_git_snapshots_for_repo_postgres(&tx, &repo_id).await?;

        entities::repository_invite::Entity::delete_many()
            .filter(entities::repository_invite::Column::RepoId.eq(repo_id.clone()))
            .exec(&tx)
            .await
            .map_err(ApiError::internal)?;
        entities::repository_member::Entity::delete_many()
            .filter(entities::repository_member::Column::RepoId.eq(repo_id.clone()))
            .exec(&tx)
            .await
            .map_err(ApiError::internal)?;
        entities::repository::Entity::delete_by_id(repo_id.clone())
            .exec(&tx)
            .await
            .map_err(ApiError::internal)?;

        save_repo_effects(&tx, &mutation.effects).await?;
        queue_pending_source_blob_deletion_rows(&tx, request_git_snapshots).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation.result)
    }
}

async fn refund_open_request_stakes_for_repo_postgres<C>(
    conn: &C,
    repo_id: &str,
    now_unix: u64,
) -> Result<(), ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let requests = requests_by_repo_id(conn, repo_id).await?;
    for request in requests
        .iter()
        .filter(|request| should_refund_on_repo_delete(request))
    {
        let stake_entry = request_stake_debit_entry_for_request_id(conn, &request.id)
            .await?
            .ok_or_else(|| {
                ApiError::internal_message("request stake debit ledger entry is missing")
            })?;
        let ledger_entry_id = repo_delete_refund_ledger_entry_id(&stake_entry.id);
        ensure_repo_delete_refund_ledger_entry_available_postgres(conn, &ledger_entry_id).await?;
        let account = credit_account_by_user_id(conn, &request.author_user_id)
            .await?
            .ok_or_else(|| {
                ApiError::internal_message("request author credit account is missing")
            })?;
        let (account, ledger_entry) =
            refund_open_request_stake(&account, request, ledger_entry_id, now_unix)?;
        save_credit_account_row(conn, &account).await?;
        insert_credit_ledger_entry_row(conn, &ledger_entry).await?;
    }
    Ok(())
}

async fn request_git_snapshots_for_repo_postgres<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Vec<SourceBlob>, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    Ok(requests_by_repo_id(conn, repo_id)
        .await?
        .into_iter()
        .filter_map(|request| request.git_snapshot)
        .collect())
}

fn should_refund_on_repo_delete(request: &Request) -> bool {
    request.stake_credits > 0 && request.settlement.is_none()
}

fn refund_open_request_stake(
    account: &UserCreditAccount,
    request: &Request,
    ledger_entry_id: String,
    now_unix: u64,
) -> Result<(UserCreditAccount, CreditLedgerEntry), ApiError> {
    let balance_credits = account
        .balance_credits
        .checked_add(request.stake_credits)
        .ok_or_else(|| ApiError::bad_request("credit balance overflow"))?;
    let amount_credits = credit_u32_to_i32(request.stake_credits)?;
    credit_u32_to_i32(balance_credits)?;
    Ok((
        UserCreditAccount {
            user_id: account.user_id.clone(),
            balance_credits,
        },
        CreditLedgerEntry {
            id: ledger_entry_id,
            user_id: request.author_user_id.clone(),
            request_id: Some(request.id.clone()),
            kind: CreditLedgerEntryKind::StakeRefund,
            amount_credits,
            created_at_unix: now_unix,
        },
    ))
}

async fn ensure_repo_delete_refund_ledger_entry_available_postgres<C>(
    conn: &C,
    ledger_entry_id: &str,
) -> Result<(), ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    if credit_ledger_entry_by_id(conn, ledger_entry_id)
        .await?
        .is_some()
    {
        Err(ApiError::conflict("credit ledger entry already exists"))
    } else {
        Ok(())
    }
}

fn repo_delete_refund_ledger_entry_id(stake_ledger_entry_id: &str) -> String {
    format!("repo_delete_refund:{stake_ledger_entry_id}")
}

fn credit_u32_to_i32(value: u32) -> Result<i32, ApiError> {
    i32::try_from(value).map_err(|_| ApiError::bad_request("credit amount exceeds i32 range"))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
