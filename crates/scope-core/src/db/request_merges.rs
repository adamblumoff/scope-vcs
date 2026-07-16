use super::{
    MetadataStore, RepositoryMutation, acquire_aggregate_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    entities, repository_from_model,
    repository_rows::save_repository_delta,
    request_access::{ensure_request_maintainer, ensure_user_exists},
    request_rows::{
        credit_account_by_user_id, credit_ledger_entry_by_id, insert_credit_ledger_entry_row,
        insert_request_event_row, request_by_id, request_event_by_id, save_credit_account_row,
        save_request_row,
    },
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
    pub async fn merge_request_with_repository_mutation<R, F>(
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

        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;

        let repo = entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
        let mut repo = repository_from_model(&tx, repo).await?;
        let before_repo = repo.clone();

        acquire_aggregate_lock(&tx, "request", &input.request_id).await?;
        let request = request_by_id(&tx, &input.request_id)
            .await?
            .ok_or_else(|| ApiError::not_found("request not found"))?;
        if request.repo_id != repo.record.id {
            return Err(ApiError::not_found("request not found"));
        }
        acquire_aggregate_lock(&tx, "user-credit", &request.author_user_id).await?;
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
        if let Some(account) = credit_account_by_user_id(&tx, &request.author_user_id).await? {
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

        save_repository_delta(&tx, &before_repo, &repo).await?;
        save_request_row(&tx, &request_mutation.request).await?;
        insert_request_event_row(&tx, &request_mutation.merged_event).await?;
        insert_request_event_row(&tx, &request_mutation.settled_event).await?;
        if let Some(account) = &request_mutation.account {
            save_credit_account_row(&tx, account).await?;
        }
        for entry in &request_mutation.ledger_entries {
            insert_credit_ledger_entry_row(&tx, entry).await?;
        }
        if !repository_mutation.orphan_objects.is_empty() {
            queue_pending_source_blob_deletion_rows(&tx, repository_mutation.orphan_objects)
                .await?;
        }
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(RequestMergeRepositoryMutation {
            repository_result: repository_mutation.result,
            request: request_mutation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        policy::Visibility,
        requests::{Request, RequestActorRole, RequestAudience, RequestState},
        store::{AppCatalog, RepoPublicationState, StoredRepository, UserAccount, app_catalog},
    };

    #[tokio::test]
    async fn combined_merge_rolls_back_request_when_repo_mutation_fails() {
        let store = store_with_owner_request().await;

        let error = store
            .merge_request_with_repository_mutation("owner", "repo", merge_input(), |repo| {
                repo.record.change_version = 99;
                Err::<RepositoryMutation<()>, ApiError>(ApiError::conflict(
                    "simulated repo conflict",
                ))
            })
            .await
            .unwrap_err();

        assert!(error.message.contains("simulated repo conflict"));
        let repo = store
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repo.record.change_version, 1);
        let request = store.request_for_tests("req_1").await.unwrap().unwrap();
        assert_eq!(request.state, RequestState::Submitted);
        assert!(request.disposition.is_none());
        assert!(request.settlement.is_none());
        assert!(store.request_events_for_tests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn combined_merge_commits_repo_and_request_together() {
        let store = store_with_owner_request().await;

        let mutation = store
            .merge_request_with_repository_mutation("owner", "repo", merge_input(), |repo| {
                repo.record.change_version = 2;
                Ok(RepositoryMutation::new("applied"))
            })
            .await
            .unwrap();

        assert_eq!(mutation.repository_result, "applied");
        assert_eq!(mutation.request.request.state, RequestState::Resolved);
        let repo = store
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repo.record.change_version, 2);
        let request = store.request_for_tests("req_1").await.unwrap().unwrap();
        assert_eq!(request.state, RequestState::Resolved);
        assert_eq!(store.request_events_for_tests().await.unwrap().len(), 2);
    }

    async fn store_with_owner_request() -> MetadataStore {
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
        let mut repo = StoredRepository::new(&owner, "repo", Visibility::Public).unwrap();
        repo.record.publication_state = RepoPublicationState::Published;

        let mut catalog = app_catalog();
        catalog.users.insert(owner.id.clone(), owner);
        catalog.repositories.insert(repo.record.id.clone(), repo);
        catalog.requests.insert(
            "req_1".to_string(),
            Request {
                id: "req_1".to_string(),
                repo_id: "owner/repo".to_string(),
                name: "owner-request".to_string(),
                author_user_id: "user_owner".to_string(),
                author_role: RequestActorRole::Owner,
                audience: RequestAudience::Private,
                base_main_oid: "main_a".to_string(),
                head_oid: "head_a".to_string(),
                git_snapshot: None,
                title: "Owner request".to_string(),
                description_markdown: String::new(),
                state: RequestState::Submitted,
                activity_version: 1,
                stake_credits: 0,
                disposition: None,
                settlement: None,
                created_at_unix: 1,
                updated_at_unix: 1,
                resolved_at_unix: None,
            },
        );
        catalog
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
