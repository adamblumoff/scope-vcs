#[cfg(any(test, feature = "memory-metadata"))]
use super::cleanup_queue::queue_pending_source_blob_deletions;
use super::{
    MetadataStore, MetadataStoreInner, RepositoryMutation, acquire_metadata_write_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    entities, repository_from_model,
    repository_rows::save_repository_row,
    request_rows::{
        credit_account_by_user_id, credit_ledger_entry_by_id, insert_credit_ledger_entry_row,
        insert_request_event_row, request_by_id, request_event_by_id, save_credit_account_row,
        save_request_row,
    },
    requests::{ensure_request_maintainer, ensure_user_exists},
    run_api_db_on,
};
use crate::{
    domain::{
        requests::{MergeRequestInput, MergeRequestMutation, merge_request},
        store::{StoredRepository, repo_id},
    },
    error::ApiError,
};
use sea_orm::{EntityTrait, TransactionTrait};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug)]
pub struct RequestMergeRepositoryMutation<R> {
    pub repository_result: R,
    pub request: MergeRequestMutation,
}

impl MetadataStore {
    pub fn merge_request_with_repository_mutation<R, F>(
        &self,
        owner: &str,
        name: &str,
        input: MergeRequestInput,
        repo_op: F,
    ) -> Result<RequestMergeRepositoryMutation<R>, ApiError>
    where
        R: Send + 'static,
        F: FnOnce(&mut StoredRepository) -> Result<RepositoryMutation<R>, ApiError>
            + Send
            + 'static,
    {
        let repo_id = repo_id(owner, name);
        let owner = owner.to_string();
        let name = name.to_string();

        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;

                    let repo = entities::repository::Entity::find_by_id(repo_id)
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| {
                            ApiError::not_found(format!("repo {owner}/{name} not found"))
                        })?;
                    let mut repo = repository_from_model(&tx, repo).await?;

                    let request = request_by_id(&tx, &input.request_id)
                        .await?
                        .ok_or_else(|| ApiError::not_found("request not found"))?;
                    if request.repo_id != repo.record.id {
                        return Err(ApiError::not_found("request not found"));
                    }
                    ensure_user_exists(&tx, &input.actor_user_id).await?;
                    ensure_request_maintainer(&repo, &input.actor_user_id)?;

                    let mut requests = BTreeMap::from([(request.id.clone(), request.clone())]);
                    let mut events = BTreeMap::new();
                    for event_id in [&input.event_id, &input.settlement_event_id] {
                        if let Some(event) = request_event_by_id(&tx, event_id).await? {
                            events.insert(event.id.clone(), event);
                        }
                    }
                    let mut accounts = BTreeMap::new();
                    if let Some(account) =
                        credit_account_by_user_id(&tx, &request.author_user_id).await?
                    {
                        accounts.insert(account.user_id.clone(), account);
                    }
                    let mut ledger_entries = BTreeMap::new();
                    for entry_id in [
                        input.refund_ledger_entry_id.as_deref(),
                        input.reward_ledger_entry_id.as_deref(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if let Some(entry) = credit_ledger_entry_by_id(&tx, entry_id).await? {
                            ledger_entries.insert(entry.id.clone(), entry);
                        }
                    }

                    let request_mutation = merge_request(
                        &mut requests,
                        &mut events,
                        &mut accounts,
                        &mut ledger_entries,
                        input,
                    )?;
                    let repository_mutation = repo_op(&mut repo)?;

                    save_repository_row(&tx, &repo).await?;
                    save_request_row(&tx, &request_mutation.request).await?;
                    insert_request_event_row(&tx, &request_mutation.merged_event).await?;
                    insert_request_event_row(&tx, &request_mutation.settled_event).await?;
                    if let Some(account) = &request_mutation.account {
                        save_credit_account_row(&tx, account).await?;
                    }
                    for entry in &request_mutation.ledger_entries {
                        insert_credit_ledger_entry_row(&tx, entry).await?;
                    }
                    if !repository_mutation.source_blobs_to_delete.is_empty() {
                        queue_pending_source_blob_deletion_rows(
                            &tx,
                            repository_mutation.source_blobs_to_delete,
                        )
                        .await?;
                    }
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(RequestMergeRepositoryMutation {
                        repository_result: repository_mutation.result,
                        request: request_mutation,
                    })
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => {
                self.update(move |catalog| {
                    let request = catalog
                        .requests
                        .get(&input.request_id)
                        .ok_or_else(|| ApiError::not_found("request not found"))?;
                    if request.repo_id != repo_id {
                        return Err(ApiError::not_found("request not found"));
                    }
                    if !catalog.users.contains_key(&input.actor_user_id) {
                        return Err(ApiError::not_found("user not found"));
                    }
                    let repo = catalog.repositories.get(&repo_id).ok_or_else(|| {
                        ApiError::not_found(format!("repo {owner}/{name} not found"))
                    })?;
                    ensure_request_maintainer(repo, &input.actor_user_id)?;

                    let request_mutation = merge_request(
                        &mut catalog.requests,
                        &mut catalog.request_events,
                        &mut catalog.user_credit_accounts,
                        &mut catalog.credit_ledger_entries,
                        input,
                    )?;
                    let repo = catalog.repositories.get_mut(&repo_id).ok_or_else(|| {
                        ApiError::not_found(format!("repo {owner}/{name} not found"))
                    })?;
                    let repository_mutation = repo_op(repo)?;
                    queue_pending_source_blob_deletions(
                        &mut catalog.pending_source_blob_deletions,
                        repository_mutation.source_blobs_to_delete,
                    );
                    Ok(RequestMergeRepositoryMutation {
                        repository_result: repository_mutation.result,
                        request: request_mutation,
                    })
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        policy::Visibility,
        requests::{
            RequestActorRole, RequestBaseAudience, RequestState, SubmitRequestInput,
            canonical_request_ref,
        },
        store::{AppCatalog, RepoPublicationState, StoredRepository, UserAccount, app_catalog},
    };

    #[test]
    fn combined_merge_rolls_back_request_when_repo_mutation_fails() {
        let store = store_with_owner_request();

        let error = store
            .merge_request_with_repository_mutation("owner", "repo", merge_input(), |repo| {
                repo.record.change_version = 99;
                Err::<RepositoryMutation<()>, ApiError>(ApiError::conflict(
                    "simulated repo conflict",
                ))
            })
            .unwrap_err();

        assert!(error.message.contains("simulated repo conflict"));
        store
            .read(|catalog| {
                let repo = catalog.repositories.get("owner/repo").unwrap();
                assert_eq!(repo.record.change_version, 1);
                let request = catalog.requests.get("req_1").unwrap();
                assert_eq!(request.state, RequestState::Submitted);
                assert!(request.disposition.is_none());
                assert!(request.settlement.is_none());
                assert_eq!(catalog.request_events.len(), 1);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn combined_merge_commits_repo_and_request_together() {
        let store = store_with_owner_request();

        let mutation = store
            .merge_request_with_repository_mutation("owner", "repo", merge_input(), |repo| {
                repo.record.change_version = 2;
                Ok(RepositoryMutation::new("applied"))
            })
            .unwrap();

        assert_eq!(mutation.repository_result, "applied");
        assert_eq!(mutation.request.request.state, RequestState::Resolved);
        store
            .read(|catalog| {
                let repo = catalog.repositories.get("owner/repo").unwrap();
                assert_eq!(repo.record.change_version, 2);
                let request = catalog.requests.get("req_1").unwrap();
                assert_eq!(request.state, RequestState::Resolved);
                assert_eq!(catalog.request_events.len(), 3);
                Ok(())
            })
            .unwrap();
    }

    fn store_with_owner_request() -> MetadataStore {
        let store = MetadataStore::memory(catalog_with_repo());
        store.submit_request(owner_submit_input()).unwrap();
        store
    }

    fn catalog_with_repo() -> AppCatalog {
        let owner = UserAccount {
            id: "user_owner".to_string(),
            handle: "owner".to_string(),
            email: "owner@example.com".to_string(),
            email_verified: true,
        };
        let mut repo = StoredRepository::new(&owner, "repo", Visibility::Public).unwrap();
        repo.record.publication_state = RepoPublicationState::Published;

        let mut catalog = app_catalog();
        catalog.users.insert(owner.id.clone(), owner);
        catalog.repositories.insert(repo.record.id.clone(), repo);
        catalog
    }

    fn owner_submit_input() -> SubmitRequestInput {
        SubmitRequestInput {
            id: "req_1".to_string(),
            repo_id: "owner/repo".to_string(),
            author_user_id: "user_owner".to_string(),
            author_role: RequestActorRole::Owner,
            base_audience: RequestBaseAudience::Private,
            target_branch: "main".to_string(),
            request_ref: canonical_request_ref("req_1"),
            base_main_oid: "main_a".to_string(),
            head_oid: "head_a".to_string(),
            title: "Owner request".to_string(),
            stake_credits: 0,
            stake_ledger_entry_id: None,
            event_id: "event_created".to_string(),
            now_unix: 1,
        }
    }

    fn merge_input() -> MergeRequestInput {
        MergeRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_owner".to_string(),
            expected_main_oid: "main_a".to_string(),
            current_main_oid: "main_a".to_string(),
            expected_head_oid: "head_a".to_string(),
            event_id: "event_merged".to_string(),
            settlement_event_id: "event_settled".to_string(),
            refund_ledger_entry_id: None,
            reward_ledger_entry_id: None,
            body: None,
            now_unix: 2,
        }
    }
}
