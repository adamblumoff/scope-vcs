#[cfg(any(test, feature = "memory-metadata"))]
use super::cleanup_queue::remove_matching_pending_repo_storage_cleanup;
#[cfg(any(test, feature = "memory-metadata"))]
use super::repo_effects::apply_repo_effects;
use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock,
    cleanup_queue::{complete_pending_repo_storage_cleanup, pending_repo_storage_cleanup_exists},
    entities,
    repo_effects::save_repo_effects,
    repository_from_model,
    repository_rows::insert_repository,
    request_rows::{
        credit_account_by_user_id, credit_ledger_entry_by_id, insert_credit_ledger_entry_row,
        request_stake_debit_entry_for_request_id, requests_by_repo_id, save_credit_account_row,
    },
    run_api_db_on,
};
use crate::domain::{
    policy::Visibility,
    repo_actions::{create_repo as create_repo_command, delete_repo as delete_repo_command},
    requests::{CreditLedgerEntry, CreditLedgerEntryKind, Request, UserCreditAccount},
    store::{FirstPushToken, GitPushToken, StoredRepository, repo_id},
};
use crate::error::ApiError;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use std::sync::Arc;

impl MetadataStore {
    pub fn create_repo_with_init_tokens<F>(
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
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let owner = entities::user::Entity::find_by_id(owner_user_id)
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| {
                            ApiError::internal_message("signed-in user was not persisted")
                        })?
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
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let owner = catalog.users.get(&owner_user_id).cloned().ok_or_else(|| {
                    ApiError::internal_message("signed-in user was not persisted")
                })?;
                let mutation = create_repo_command(
                    &owner,
                    &name,
                    default_visibility,
                    first_push_token,
                    git_push_token,
                )?;
                let repo = mutation.result;
                if catalog.repositories.contains_key(&repo.record.id) {
                    return Err(ApiError::conflict(format!(
                        "repo {} already exists",
                        repo.record.id
                    )));
                }

                let had_pending_cleanup = remove_matching_pending_repo_storage_cleanup(
                    &mut catalog.pending_repo_storage_deletions,
                    &repo.record.id,
                );
                if had_pending_cleanup {
                    cleanup_pending_storage(&repo.record.owner_handle, &repo.record.name)?;
                }

                catalog
                    .repositories
                    .insert(repo.record.id.clone(), repo.clone());
                apply_repo_effects(catalog, mutation.effects);
                Ok(repo)
            }),
        }
    }

    pub fn delete_repo(&self, owner: &str, name: &str, user_id: &str) -> Result<String, ApiError> {
        let repo_id = repo_id(owner, name);
        let owner = owner.to_string();
        let name = name.to_string();
        let user_id = user_id.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let repo = entities::repository::Entity::find_by_id(repo_id.clone())
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| {
                            crate::domain::repo_actions::hidden_repo_not_found(&owner, &name)
                        })?;
                    let repo = repository_from_model(&tx, repo).await?;
                    let mutation = delete_repo_command(&repo, &user_id, &owner, &name)?;
                    refund_open_request_stakes_for_repo_postgres(&tx, &repo_id, unix_now()).await?;

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
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(mutation.result)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog.repositories.get(&repo_id).ok_or_else(|| {
                    crate::domain::repo_actions::hidden_repo_not_found(&owner, &name)
                })?;
                let mutation = delete_repo_command(repo, &user_id, &owner, &name)?;

                refund_open_request_stakes_for_repo_memory(catalog, &repo_id, unix_now())?;
                catalog
                    .repositories
                    .remove(&repo_id)
                    .expect("repo was already checked");
                remove_request_facts_for_repo(catalog, &repo_id);
                apply_repo_effects(catalog, mutation.effects);
                Ok(mutation.result)
            }),
        }
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

#[cfg(any(test, feature = "memory-metadata"))]
fn refund_open_request_stakes_for_repo_memory(
    catalog: &mut crate::domain::store::AppCatalog,
    repo_id: &str,
    now_unix: u64,
) -> Result<(), ApiError> {
    let requests = catalog
        .requests
        .values()
        .filter(|request| request.repo_id == repo_id)
        .cloned()
        .collect::<Vec<_>>();
    for request in requests
        .iter()
        .filter(|request| should_refund_on_repo_delete(request))
    {
        let stake_entry_id = request_stake_debit_entry_id_for_request_memory(catalog, &request.id)?;
        let ledger_entry_id = repo_delete_refund_ledger_entry_id(&stake_entry_id);
        ensure_repo_delete_refund_ledger_entry_available_memory(catalog, &ledger_entry_id)?;
        let account = catalog
            .user_credit_accounts
            .get(&request.author_user_id)
            .ok_or_else(|| {
                ApiError::internal_message("request author credit account is missing")
            })?;
        let (account, ledger_entry) =
            refund_open_request_stake(account, request, ledger_entry_id, now_unix)?;
        catalog
            .user_credit_accounts
            .insert(account.user_id.clone(), account);
        catalog
            .credit_ledger_entries
            .insert(ledger_entry.id.clone(), ledger_entry);
    }
    Ok(())
}

#[cfg(any(test, feature = "memory-metadata"))]
fn request_stake_debit_entry_id_for_request_memory(
    catalog: &crate::domain::store::AppCatalog,
    request_id: &str,
) -> Result<String, ApiError> {
    catalog
        .credit_ledger_entries
        .values()
        .filter(|entry| {
            entry.request_id.as_deref() == Some(request_id)
                && entry.kind == CreditLedgerEntryKind::RequestStakeDebit
        })
        .map(|entry| entry.id.clone())
        .next()
        .ok_or_else(|| ApiError::internal_message("request stake debit ledger entry is missing"))
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

#[cfg(any(test, feature = "memory-metadata"))]
fn ensure_repo_delete_refund_ledger_entry_available_memory(
    catalog: &crate::domain::store::AppCatalog,
    ledger_entry_id: &str,
) -> Result<(), ApiError> {
    if catalog.credit_ledger_entries.contains_key(ledger_entry_id) {
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

#[cfg(any(test, feature = "memory-metadata"))]
fn remove_request_facts_for_repo(catalog: &mut crate::domain::store::AppCatalog, repo_id: &str) {
    let request_ids = catalog
        .requests
        .values()
        .filter(|request| request.repo_id == repo_id)
        .map(|request| request.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    if request_ids.is_empty() {
        return;
    }

    catalog
        .requests
        .retain(|_, request| !request_ids.contains(&request.id));
    catalog
        .request_events
        .retain(|_, event| !request_ids.contains(&event.request_id));
    for entry in catalog.credit_ledger_entries.values_mut() {
        if entry
            .request_id
            .as_ref()
            .is_some_and(|request_id| request_ids.contains(request_id))
        {
            entry.request_id = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        requests::{
            CreditLedgerEntry, CreditLedgerEntryKind, Request, RequestActorRole,
            RequestBaseAudience, RequestEvent, RequestEventKind, RequestState, UserCreditAccount,
        },
        store::{AppCatalog, RepoPublicationState, UserAccount, app_catalog},
    };

    #[test]
    fn memory_delete_repo_refunds_open_request_stake() {
        let store = MetadataStore::memory(catalog_with_open_request(0, "ledger_stake"));
        store
            .delete_repo("owner", "repo", "user_owner")
            .expect("repo deletes");

        store
            .read(|catalog: &AppCatalog| {
                assert!(catalog.repositories.is_empty());
                assert!(catalog.requests.is_empty());
                assert!(catalog.request_events.is_empty());
                assert_eq!(
                    catalog
                        .user_credit_accounts
                        .get("user_public")
                        .unwrap()
                        .balance_credits,
                    10
                );
                assert_eq!(
                    catalog
                        .credit_ledger_entries
                        .get("ledger_stake")
                        .unwrap()
                        .request_id,
                    None
                );
                let refund = catalog
                    .credit_ledger_entries
                    .get("repo_delete_refund:ledger_stake")
                    .expect("refund ledger entry is written");
                assert_eq!(refund.kind, CreditLedgerEntryKind::StakeRefund);
                assert_eq!(refund.amount_credits, 10);
                assert_eq!(refund.request_id, None);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn memory_delete_repo_rejects_refund_ledger_collision_without_mutating_metadata() {
        let mut catalog = catalog_with_open_request(0, "ledger_stake");
        catalog.credit_ledger_entries.insert(
            "repo_delete_refund:ledger_stake".to_string(),
            CreditLedgerEntry {
                id: "repo_delete_refund:ledger_stake".to_string(),
                user_id: "user_public".to_string(),
                request_id: None,
                kind: CreditLedgerEntryKind::AdminAdjustment,
                amount_credits: 99,
                created_at_unix: 9,
            },
        );

        let store = MetadataStore::memory(catalog);
        assert!(store.delete_repo("owner", "repo", "user_owner").is_err());

        store
            .read(|catalog: &AppCatalog| {
                assert!(catalog.repositories.contains_key("owner/repo"));
                assert!(catalog.requests.contains_key("req_1"));
                assert_eq!(
                    catalog
                        .user_credit_accounts
                        .get("user_public")
                        .unwrap()
                        .balance_credits,
                    0
                );
                assert_eq!(
                    catalog
                        .credit_ledger_entries
                        .get("repo_delete_refund:ledger_stake")
                        .unwrap()
                        .amount_credits,
                    99
                );
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn memory_delete_repo_rejects_refund_balance_above_persisted_range() {
        let store =
            MetadataStore::memory(catalog_with_open_request(i32::MAX as u32, "ledger_stake"));

        assert!(store.delete_repo("owner", "repo", "user_owner").is_err());

        store
            .read(|catalog: &AppCatalog| {
                assert!(catalog.repositories.contains_key("owner/repo"));
                assert!(catalog.requests.contains_key("req_1"));
                assert_eq!(
                    catalog
                        .user_credit_accounts
                        .get("user_public")
                        .unwrap()
                        .balance_credits,
                    i32::MAX as u32
                );
                assert!(
                    !catalog
                        .credit_ledger_entries
                        .contains_key("repo_delete_refund:ledger_stake")
                );
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn memory_delete_repo_refund_id_survives_request_id_reuse_history() {
        let mut catalog = catalog_with_open_request(0, "ledger_stake_2");
        catalog.credit_ledger_entries.insert(
            "repo_delete_refund:req_1".to_string(),
            CreditLedgerEntry {
                id: "repo_delete_refund:req_1".to_string(),
                user_id: "user_public".to_string(),
                request_id: None,
                kind: CreditLedgerEntryKind::StakeRefund,
                amount_credits: 10,
                created_at_unix: 8,
            },
        );

        let store = MetadataStore::memory(catalog);
        store
            .delete_repo("owner", "repo", "user_owner")
            .expect("repo deletes despite old request-id-shaped refund");

        store
            .read(|catalog: &AppCatalog| {
                assert!(
                    catalog
                        .credit_ledger_entries
                        .contains_key("repo_delete_refund:req_1")
                );
                assert!(
                    catalog
                        .credit_ledger_entries
                        .contains_key("repo_delete_refund:ledger_stake_2")
                );
                Ok(())
            })
            .unwrap();
    }

    fn catalog_with_open_request(balance_credits: u32, stake_ledger_entry_id: &str) -> AppCatalog {
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
        catalog.requests.insert("req_1".to_string(), open_request());
        catalog.request_events.insert(
            "event_created".to_string(),
            RequestEvent {
                id: "event_created".to_string(),
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                kind: RequestEventKind::Created,
                body: None,
                old_head_oid: None,
                new_head_oid: Some("head".to_string()),
                created_at_unix: 10,
            },
        );
        catalog.user_credit_accounts.insert(
            "user_public".to_string(),
            UserCreditAccount {
                user_id: "user_public".to_string(),
                balance_credits,
            },
        );
        catalog.credit_ledger_entries.insert(
            stake_ledger_entry_id.to_string(),
            CreditLedgerEntry {
                id: stake_ledger_entry_id.to_string(),
                user_id: "user_public".to_string(),
                request_id: Some("req_1".to_string()),
                kind: CreditLedgerEntryKind::RequestStakeDebit,
                amount_credits: -10,
                created_at_unix: 10,
            },
        );
        catalog
    }

    fn open_request() -> Request {
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
}
