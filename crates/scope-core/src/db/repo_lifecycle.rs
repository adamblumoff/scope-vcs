use super::{
    MetadataStore, acquire_aggregate_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    cleanup_queue::{
        claim_pending_repo_storage_cleanup, complete_claimed_repo_storage_cleanup,
        pending_repo_storage_cleanup_exists,
    },
    entities,
    object_references::delete_object_references_for_objects,
    repo_effects::save_repo_effects,
    repository_from_model,
    repository_rows::insert_repository,
    request_change_block_rows::change_blocks_for_request_ids,
    request_rows::{
        credit_account_by_user_id, credit_ledger_entry_by_id, insert_credit_ledger_entry_row,
        requests_by_repo_id, save_credit_account_row,
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
        let owner = entities::user::Entity::find_by_id(owner_user_id)
            .one(self.db.as_ref())
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
        let db = Arc::clone(&self.db);
        let repo_id = repo.record.id.clone();
        self.with_repo_storage_lock(&repo_id, move || async move {
            let claim_tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
            acquire_aggregate_lock(&claim_tx, "repository", &repo.record.id).await?;
            ensure_repository_absent(&claim_tx, &repo.record.id).await?;
            let cleanup_claim =
                claim_pending_repo_storage_cleanup(&claim_tx, &repo.record.id).await?;
            claim_tx.commit().await.map_err(ApiError::internal)?;

            if cleanup_claim.is_some() {
                cleanup_pending_storage(&repo.record.owner_handle, &repo.record.name)?;
            }

            let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
            acquire_aggregate_lock(&tx, "repository", &repo.record.id).await?;
            ensure_repository_absent(&tx, &repo.record.id).await?;
            match cleanup_claim {
                Some(claim) => {
                    complete_claimed_repo_storage_cleanup(&tx, &repo.record.id, &claim).await?
                }
                None if pending_repo_storage_cleanup_exists(&tx, &repo.record.id).await? => {
                    return Err(ApiError::conflict(
                        "repository storage cleanup changed during creation; retry",
                    ));
                }
                None => {}
            }

            insert_repository(&tx, &repo).await?;
            save_repo_effects(&tx, &mutation.effects).await?;
            tx.commit().await.map_err(ApiError::internal)?;
            Ok(repo)
        })
        .await
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
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let repo = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| crate::domain::repo_actions::hidden_repo_not_found(&owner, &name))?;
        let repo = repository_from_model(&tx, repo).await?;
        let mutation = delete_repo_command(&repo, &user_id, &owner, &name)?;
        let repository_objects = repo.source_blobs();
        let requests = lock_requests_for_repo_postgres(&tx, &repo_id).await?;
        lock_request_credit_accounts_for_repo_postgres(&tx, &requests).await?;
        refund_open_request_stakes_for_repo_postgres(&tx, &requests, unix_now()).await?;
        let request_ids = requests
            .iter()
            .map(|request| request.id.clone())
            .collect::<Vec<_>>();
        let change_blocks = change_blocks_for_request_ids(&tx, &request_ids).await?;
        let request_git_snapshots = request_git_snapshots_for_repo(&requests, &change_blocks);
        delete_object_references_for_objects(
            &tx,
            repository_objects
                .iter()
                .chain(request_git_snapshots.iter()),
        )
        .await?;

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

async fn ensure_repository_absent<C>(conn: &C, repo_id: &str) -> Result<(), ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    if entities::repository::Entity::find_by_id(repo_id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .is_some()
    {
        Err(ApiError::conflict(format!("repo {repo_id} already exists")))
    } else {
        Ok(())
    }
}

async fn refund_open_request_stakes_for_repo_postgres<C>(
    conn: &C,
    requests: &[Request],
    now_unix: u64,
) -> Result<(), ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    for request in requests
        .iter()
        .filter(|request| should_refund_on_repo_delete(request))
    {
        let ledger_entry_id = repo_delete_refund_ledger_entry_id(request)?;
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

async fn lock_request_credit_accounts_for_repo_postgres<C>(
    conn: &C,
    requests: &[Request],
) -> Result<(), ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let mut author_ids = requests
        .iter()
        .filter(|request| should_refund_on_repo_delete(request))
        .map(|request| request.author_user_id.as_str())
        .collect::<Vec<_>>();
    author_ids.sort_unstable();
    author_ids.dedup();
    for author_id in author_ids {
        acquire_aggregate_lock(conn, "user-credit", author_id).await?;
    }
    Ok(())
}

async fn lock_requests_for_repo_postgres<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Vec<Request>, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let mut request_ids = requests_by_repo_id(conn, repo_id)
        .await?
        .into_iter()
        .map(|request| request.id)
        .collect::<Vec<_>>();
    request_ids.sort();

    let mut requests = Vec::with_capacity(request_ids.len());
    for request_id in request_ids {
        acquire_aggregate_lock(conn, "request", &request_id).await?;
        if let Some(request) = super::request_rows::request_by_id(conn, &request_id).await?
            && request.repo_id == repo_id
        {
            requests.push(request);
        }
    }
    Ok(requests)
}

fn request_git_snapshots_for_repo(
    requests: &[Request],
    change_blocks: &[crate::domain::requests::RequestChangeBlock],
) -> Vec<SourceBlob> {
    let mut snapshots = requests
        .iter()
        .filter_map(|request| request.git_snapshot.clone())
        .chain(change_blocks.iter().map(|block| block.git_snapshot.clone()))
        .collect::<Vec<_>>();
    snapshots.sort_by(|left, right| left.object_key.cmp(&right.object_key));
    snapshots.dedup_by(|left, right| left.object_key == right.object_key);
    snapshots
}

fn should_refund_on_repo_delete(request: &Request) -> bool {
    request.current_stake_credits > 0
}

fn refund_open_request_stake(
    account: &UserCreditAccount,
    request: &Request,
    ledger_entry_id: String,
    now_unix: u64,
) -> Result<(UserCreditAccount, CreditLedgerEntry), ApiError> {
    let balance_credits = account
        .balance_credits
        .checked_add(request.current_stake_credits)
        .ok_or_else(|| ApiError::bad_request("credit balance overflow"))?;
    let amount_credits = credit_u32_to_i32(request.current_stake_credits)?;
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
            kind: CreditLedgerEntryKind::ReviewStakeRefund,
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

fn repo_delete_refund_ledger_entry_id(request: &Request) -> Result<String, ApiError> {
    let ready_at_unix = request.ready_at_unix.ok_or_else(|| {
        ApiError::internal_message("staked request is missing its current ready time")
    })?;
    Ok(format!("repo_delete_refund:{}:{ready_at_unix}", request.id))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::tokens::{generate_first_push_token, generate_git_push_token},
        domain::store::{RepoStorageCleanup, UserAccount, app_catalog},
    };
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pending_storage_cleanup_reserves_repo_name_until_recreation_commits() {
        let target = super::super::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let mut catalog = app_catalog();
        catalog.users.insert(
            "user_owner".to_string(),
            UserAccount {
                id: "user_owner".to_string(),
                handle: "owner".to_string(),
                email: "owner@example.com".to_string(),
                email_verified: true,
            },
        );
        store.seed_catalog_for_tests(catalog).unwrap();
        super::super::cleanup_queue::queue_pending_repo_storage_cleanup_row(
            store.db.as_ref(),
            RepoStorageCleanup {
                owner_handle: "owner".to_string(),
                repo_name: "repo".to_string(),
            },
        )
        .await
        .unwrap();

        let (cleanup_started_tx, cleanup_started_rx) = tokio::sync::oneshot::channel();
        let (release_cleanup_tx, release_cleanup_rx) = std::sync::mpsc::channel();
        let first_store = store.clone();
        let first = tokio::spawn(async move {
            let (_, first_push_token) = generate_first_push_token("user_owner").unwrap();
            let (_, git_push_token) = generate_git_push_token("user_owner").unwrap();
            first_store
                .create_repo_with_init_tokens(
                    "user_owner",
                    "repo",
                    Visibility::Private,
                    first_push_token,
                    git_push_token,
                    move |_, _| {
                        cleanup_started_tx.send(()).unwrap();
                        release_cleanup_rx.recv().unwrap();
                        Ok(())
                    },
                )
                .await
        });
        cleanup_started_rx.await.unwrap();

        let competing_cleanup_called = Arc::new(AtomicBool::new(false));
        let second_cleanup_called = Arc::clone(&competing_cleanup_called);
        let second_store = store.clone();
        let second = tokio::spawn(async move {
            let (_, first_push_token) = generate_first_push_token("user_owner").unwrap();
            let (_, git_push_token) = generate_git_push_token("user_owner").unwrap();
            second_store
                .create_repo_with_init_tokens(
                    "user_owner",
                    "repo",
                    Visibility::Private,
                    first_push_token,
                    git_push_token,
                    move |_, _| {
                        second_cleanup_called.store(true, Ordering::SeqCst);
                        Ok(())
                    },
                )
                .await
        });
        wait_until_repository_lock_is_waited_on(&store).await;
        assert!(
            !second.is_finished(),
            "competing creator must wait for the storage path lock"
        );
        release_cleanup_tx.send(()).unwrap();
        let first_result = first.await.unwrap();
        let second_result = second.await.unwrap();

        first_result.unwrap();
        assert!(
            second_result
                .unwrap_err()
                .message
                .contains("already exists")
        );
        assert!(!competing_cleanup_called.load(Ordering::SeqCst));
        assert!(
            entities::repository::Entity::find_by_id("owner/repo")
                .one(store.db.as_ref())
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn expired_creation_cleanup_claim_cannot_commit_after_worker_reclaims_it() {
        let target = super::super::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        super::super::cleanup_queue::queue_pending_repo_storage_cleanup_row(
            store.db.as_ref(),
            RepoStorageCleanup {
                owner_handle: "owner".to_string(),
                repo_name: "repo".to_string(),
            },
        )
        .await
        .unwrap();

        let claim_tx = store.db.begin().await.unwrap();
        acquire_aggregate_lock(&claim_tx, "repository", "owner/repo")
            .await
            .unwrap();
        let claim = claim_pending_repo_storage_cleanup(&claim_tx, "owner/repo")
            .await
            .unwrap()
            .unwrap();
        claim_tx.commit().await.unwrap();

        entities::repo_storage_cleanup_job::Entity::update_many()
            .filter(entities::repo_storage_cleanup_job::Column::RepoId.eq("owner/repo"))
            .col_expr(
                entities::repo_storage_cleanup_job::Column::NextRunAtUnix,
                sea_orm::sea_query::Expr::value(0_i64),
            )
            .exec(store.db.as_ref())
            .await
            .unwrap();
        let _worker_batch = store.repo_storage_cleanup_batch().await.unwrap();

        let create_tx = store.db.begin().await.unwrap();
        acquire_aggregate_lock(&create_tx, "repository", "owner/repo")
            .await
            .unwrap();
        let error = complete_claimed_repo_storage_cleanup(&create_tx, "owner/repo", &claim)
            .await
            .unwrap_err();
        assert!(error.message.contains("changed during creation"));
    }

    async fn wait_until_repository_lock_is_waited_on(store: &MetadataStore) {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let waiting = store
                    .db
                    .query_one(Statement::from_string(
                        DatabaseBackend::Postgres,
                        "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND wait_event = 'advisory') AS waiting".to_string(),
                    ))
                    .await
                    .unwrap()
                    .unwrap()
                    .try_get::<bool>("", "waiting")
                    .unwrap();
                if waiting {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("competing creator should wait for repository lock");
    }
}
