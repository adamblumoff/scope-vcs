#[cfg(any(test, feature = "memory-metadata"))]
use super::cleanup_queue::queue_pending_source_blob_deletions;
use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    entities, repository_from_model,
    request_rows::{
        credit_account_by_user_id, credit_ledger_entry_by_id, insert_credit_ledger_entry_row,
        insert_request_event_row, insert_request_row, request_by_id, request_by_ref,
        request_event_by_id, requests_by_repo_author, save_credit_account_row, save_request_row,
    },
    run_api_db_on,
};
use crate::{
    domain::{
        requests::{
            CreditAccountMutation, GrantUserCreditsInput, MergeRequestInput, MergeRequestMutation,
            RecordRequestRevisionInput, Request, RequestActorRole, RequestBaseAudience,
            RequestRevisionMutation, ResolveRequestInput, ResolveRequestMutation,
            SubmitRequestInput, SubmitRequestMutation, grant_user_credits, merge_request,
            record_request_revision, resolve_request, submit_request,
        },
        store::{RepoPublicationState, RepositoryActor, StoredRepository},
    },
    error::ApiError,
};
use sea_orm::{EntityTrait, TransactionTrait};
use std::{collections::BTreeMap, sync::Arc};

impl MetadataStore {
    pub fn request_by_ref(&self, request_ref: &str) -> Result<Option<Request>, ApiError> {
        let request_ref = request_ref.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    request_by_ref(db.as_ref(), &request_ref).await
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.read(move |catalog| {
                Ok(catalog
                    .requests
                    .values()
                    .find(|request| request.request_ref == request_ref)
                    .cloned())
            }),
        }
    }

    pub fn requests_by_repo_author(
        &self,
        repo_id: &str,
        author_user_id: &str,
    ) -> Result<Vec<Request>, ApiError> {
        let repo_id = repo_id.to_string();
        let author_user_id = author_user_id.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    requests_by_repo_author(db.as_ref(), &repo_id, &author_user_id).await
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.read(move |catalog| {
                Ok(catalog
                    .requests
                    .values()
                    .filter(|request| {
                        request.repo_id == repo_id && request.author_user_id == author_user_id
                    })
                    .cloned()
                    .collect())
            }),
        }
    }

    pub fn grant_user_credits(
        &self,
        input: GrantUserCreditsInput,
    ) -> Result<CreditAccountMutation, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    ensure_user_exists(&tx, &input.user_id).await?;

                    let mut accounts = BTreeMap::new();
                    if let Some(account) = credit_account_by_user_id(&tx, &input.user_id).await? {
                        accounts.insert(account.user_id.clone(), account);
                    }
                    let mut ledger_entries = BTreeMap::new();
                    if let Some(entry) =
                        credit_ledger_entry_by_id(&tx, &input.ledger_entry_id).await?
                    {
                        ledger_entries.insert(entry.id.clone(), entry);
                    }

                    let mutation = grant_user_credits(&mut accounts, &mut ledger_entries, input)?;
                    save_credit_account_row(&tx, &mutation.account).await?;
                    insert_credit_ledger_entry_row(&tx, &mutation.ledger_entry).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(mutation)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                if !catalog.users.contains_key(&input.user_id) {
                    return Err(ApiError::not_found("user not found"));
                }
                grant_user_credits(
                    &mut catalog.user_credit_accounts,
                    &mut catalog.credit_ledger_entries,
                    input,
                )
            }),
        }
    }

    pub fn submit_request(
        &self,
        input: SubmitRequestInput,
    ) -> Result<SubmitRequestMutation, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    ensure_user_exists(&tx, &input.author_user_id).await?;
                    let input =
                        authorize_submit_request(&repo_by_id(&tx, &input.repo_id).await?, input)?;

                    let mut requests = BTreeMap::new();
                    if let Some(request) = request_by_id(&tx, &input.id).await? {
                        requests.insert(request.id.clone(), request);
                    }
                    if let Some(request) = request_by_ref(&tx, &input.request_ref).await? {
                        requests.insert(request.id.clone(), request);
                    }
                    let mut events = BTreeMap::new();
                    if let Some(event) = request_event_by_id(&tx, &input.event_id).await? {
                        events.insert(event.id.clone(), event);
                    }
                    let mut accounts = BTreeMap::new();
                    if let Some(account) =
                        credit_account_by_user_id(&tx, &input.author_user_id).await?
                    {
                        accounts.insert(account.user_id.clone(), account);
                    }
                    let mut ledger_entries = BTreeMap::new();
                    if let Some(entry_id) = input.stake_ledger_entry_id.as_deref()
                        && let Some(entry) = credit_ledger_entry_by_id(&tx, entry_id).await?
                    {
                        ledger_entries.insert(entry.id.clone(), entry);
                    }

                    let mutation = submit_request(
                        &mut requests,
                        &mut events,
                        &mut accounts,
                        &mut ledger_entries,
                        input,
                    )?;
                    insert_request_row(&tx, &mutation.request).await?;
                    insert_request_event_row(&tx, &mutation.event).await?;
                    if let Some(account) = &mutation.account {
                        save_credit_account_row(&tx, account).await?;
                    }
                    if let Some(entry) = &mutation.ledger_entry {
                        insert_credit_ledger_entry_row(&tx, entry).await?;
                    }
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(mutation)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                if !catalog.users.contains_key(&input.author_user_id) {
                    return Err(ApiError::not_found("user not found"));
                }
                let repo = catalog
                    .repositories
                    .get(&input.repo_id)
                    .ok_or_else(|| ApiError::not_found("repo not found"))?;
                let input = authorize_submit_request(repo, input)?;
                submit_request(
                    &mut catalog.requests,
                    &mut catalog.request_events,
                    &mut catalog.user_credit_accounts,
                    &mut catalog.credit_ledger_entries,
                    input,
                )
            }),
        }
    }

    pub fn record_request_revision(
        &self,
        input: RecordRequestRevisionInput,
    ) -> Result<RequestRevisionMutation, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    ensure_user_exists(&tx, &input.actor_user_id).await?;

                    let mut requests = BTreeMap::new();
                    if let Some(request) = request_by_id(&tx, &input.request_id).await? {
                        requests.insert(request.id.clone(), request);
                    }
                    let mut events = BTreeMap::new();
                    if let Some(event) = request_event_by_id(&tx, &input.event_id).await? {
                        events.insert(event.id.clone(), event);
                    }

                    let mutation = record_request_revision(&mut requests, &mut events, input)?;
                    save_request_row(&tx, &mutation.request).await?;
                    insert_request_event_row(&tx, &mutation.event).await?;
                    if !mutation.source_blobs_to_delete.is_empty() {
                        queue_pending_source_blob_deletion_rows(
                            &tx,
                            mutation.source_blobs_to_delete.clone(),
                        )
                        .await?;
                    }
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(mutation)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let mutation = record_request_revision(
                    &mut catalog.requests,
                    &mut catalog.request_events,
                    input,
                )?;
                queue_pending_source_blob_deletions(
                    &mut catalog.pending_source_blob_deletions,
                    mutation.source_blobs_to_delete.clone(),
                );
                Ok(mutation)
            }),
        }
    }

    pub fn resolve_request(
        &self,
        input: ResolveRequestInput,
    ) -> Result<ResolveRequestMutation, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;

                    let mut requests = BTreeMap::new();
                    let request = request_by_id(&tx, &input.request_id)
                        .await?
                        .ok_or_else(|| ApiError::not_found("request not found"))?;
                    ensure_user_exists(&tx, &input.actor_user_id).await?;
                    let repo = repo_by_id(&tx, &request.repo_id).await?;
                    ensure_request_maintainer(&repo, &input.actor_user_id)?;
                    requests.insert(request.id.clone(), request.clone());
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

                    let mutation = resolve_request(
                        &mut requests,
                        &mut events,
                        &mut accounts,
                        &mut ledger_entries,
                        input,
                    )?;
                    save_request_row(&tx, &mutation.request).await?;
                    insert_request_event_row(&tx, &mutation.resolved_event).await?;
                    insert_request_event_row(&tx, &mutation.settled_event).await?;
                    if let Some(account) = &mutation.account {
                        save_credit_account_row(&tx, account).await?;
                    }
                    for entry in &mutation.ledger_entries {
                        insert_credit_ledger_entry_row(&tx, entry).await?;
                    }
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(mutation)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let request = catalog
                    .requests
                    .get(&input.request_id)
                    .ok_or_else(|| ApiError::not_found("request not found"))?;
                if !catalog.users.contains_key(&input.actor_user_id) {
                    return Err(ApiError::not_found("user not found"));
                }
                let repo = catalog
                    .repositories
                    .get(&request.repo_id)
                    .ok_or_else(|| ApiError::not_found("repo not found"))?;
                ensure_request_maintainer(repo, &input.actor_user_id)?;
                resolve_request(
                    &mut catalog.requests,
                    &mut catalog.request_events,
                    &mut catalog.user_credit_accounts,
                    &mut catalog.credit_ledger_entries,
                    input,
                )
            }),
        }
    }

    pub fn merge_request(
        &self,
        input: MergeRequestInput,
    ) -> Result<MergeRequestMutation, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;

                    let mut requests = BTreeMap::new();
                    let request = request_by_id(&tx, &input.request_id)
                        .await?
                        .ok_or_else(|| ApiError::not_found("request not found"))?;
                    ensure_user_exists(&tx, &input.actor_user_id).await?;
                    let repo = repo_by_id(&tx, &request.repo_id).await?;
                    ensure_request_maintainer(&repo, &input.actor_user_id)?;
                    requests.insert(request.id.clone(), request.clone());
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

                    let mutation = merge_request(
                        &mut requests,
                        &mut events,
                        &mut accounts,
                        &mut ledger_entries,
                        input,
                    )?;
                    save_request_row(&tx, &mutation.request).await?;
                    insert_request_event_row(&tx, &mutation.merged_event).await?;
                    insert_request_event_row(&tx, &mutation.settled_event).await?;
                    if let Some(account) = &mutation.account {
                        save_credit_account_row(&tx, account).await?;
                    }
                    for entry in &mutation.ledger_entries {
                        insert_credit_ledger_entry_row(&tx, entry).await?;
                    }
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(mutation)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let request = catalog
                    .requests
                    .get(&input.request_id)
                    .ok_or_else(|| ApiError::not_found("request not found"))?;
                if !catalog.users.contains_key(&input.actor_user_id) {
                    return Err(ApiError::not_found("user not found"));
                }
                let repo = catalog
                    .repositories
                    .get(&request.repo_id)
                    .ok_or_else(|| ApiError::not_found("repo not found"))?;
                ensure_request_maintainer(repo, &input.actor_user_id)?;
                merge_request(
                    &mut catalog.requests,
                    &mut catalog.request_events,
                    &mut catalog.user_credit_accounts,
                    &mut catalog.credit_ledger_entries,
                    input,
                )
            }),
        }
    }
}

fn authorize_submit_request(
    repo: &StoredRepository,
    mut input: SubmitRequestInput,
) -> Result<SubmitRequestInput, ApiError> {
    let (author_role, base_audience) = match repo.access_for_user_id(&input.author_user_id).actor {
        RepositoryActor::Owner => (RequestActorRole::Owner, RequestBaseAudience::Private),
        RepositoryActor::Member => (RequestActorRole::Member, RequestBaseAudience::Private),
        RepositoryActor::Public => {
            if repo.record.publication_state != RepoPublicationState::Published {
                return Err(ApiError::forbidden("published repository required"));
            }
            (RequestActorRole::Public, RequestBaseAudience::Public)
        }
    };
    input.author_role = author_role;
    input.base_audience = base_audience;
    Ok(input)
}

fn ensure_request_maintainer(repo: &StoredRepository, user_id: &str) -> Result<(), ApiError> {
    match repo.access_for_user_id(user_id).actor {
        RepositoryActor::Owner | RepositoryActor::Member => Ok(()),
        RepositoryActor::Public => Err(ApiError::forbidden("repo maintainer required")),
    }
}

async fn ensure_user_exists<C>(conn: &C, user_id: &str) -> Result<(), ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    if entities::user::Entity::find_by_id(user_id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .is_some()
    {
        Ok(())
    } else {
        Err(ApiError::not_found("user not found"))
    }
}

async fn repo_by_id<C>(conn: &C, repo_id: &str) -> Result<StoredRepository, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let repo = entities::repository::Entity::find_by_id(repo_id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("repo not found"))?;
    repository_from_model(conn, repo).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        policy::Visibility,
        requests::{RequestActorRole, RequestBaseAudience, RequestDisposition},
        store::{AppCatalog, RepoPublicationState, StoredRepository, UserAccount, app_catalog},
    };

    #[test]
    fn memory_request_submission_and_resolution_update_credit_facts() {
        let store = MetadataStore::memory(catalog_with_repo());

        store
            .grant_user_credits(GrantUserCreditsInput {
                ledger_entry_id: "ledger_grant".to_string(),
                user_id: "user_public".to_string(),
                amount_credits: 20,
                now_unix: 1,
            })
            .unwrap();
        store.submit_request(public_submit_input()).unwrap();
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
            .unwrap();
    }

    #[test]
    fn memory_public_user_cannot_choose_owner_role_to_skip_stake() {
        let store = MetadataStore::memory(catalog_with_repo());
        let mut input = public_submit_input();
        input.author_role = RequestActorRole::Owner;
        input.base_audience = RequestBaseAudience::Private;
        input.stake_credits = 0;
        input.stake_ledger_entry_id = None;

        let error = store.submit_request(input).unwrap_err();

        assert!(
            error
                .message
                .contains("public requests require credit stake")
        );
        store
            .read(|catalog| {
                assert!(catalog.requests.is_empty());
                assert!(catalog.request_events.is_empty());
                assert!(catalog.credit_ledger_entries.is_empty());
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn memory_owner_submission_derives_private_base_without_credits() {
        let store = MetadataStore::memory(catalog_with_repo());
        let mut input = public_submit_input();
        input.id = "req_owner".to_string();
        input.request_ref = "refs/scope/requests/req_owner".to_string();
        input.author_user_id = "user_owner".to_string();
        input.author_role = RequestActorRole::Public;
        input.base_audience = RequestBaseAudience::Public;
        input.stake_credits = 0;
        input.stake_ledger_entry_id = None;
        input.event_id = "event_owner".to_string();

        let mutation = store.submit_request(input).unwrap();

        assert_eq!(mutation.request.author_role, RequestActorRole::Owner);
        assert_eq!(mutation.request.base_audience, RequestBaseAudience::Private);
        assert!(mutation.account.is_none());
        assert!(mutation.ledger_entry.is_none());
    }

    #[test]
    fn memory_non_maintainer_cannot_resolve_request() {
        let store = MetadataStore::memory(catalog_with_repo());
        store
            .grant_user_credits(GrantUserCreditsInput {
                ledger_entry_id: "ledger_grant".to_string(),
                user_id: "user_public".to_string(),
                amount_credits: 20,
                now_unix: 1,
            })
            .unwrap();
        store.submit_request(public_submit_input()).unwrap();

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
            .unwrap();
    }

    #[test]
    fn postgres_request_facts_round_trip_when_database_is_configured() {
        let Some(target) = super::super::TestDatabaseTarget::from_env().unwrap() else {
            eprintln!("skipping request Postgres test; SCOPE_TEST_DATABASE_URL is not set");
            return;
        };
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        store.seed_catalog_for_tests(catalog_with_repo()).unwrap();

        store
            .grant_user_credits(GrantUserCreditsInput {
                ledger_entry_id: "ledger_grant".to_string(),
                user_id: "user_public".to_string(),
                amount_credits: 20,
                now_unix: 1,
            })
            .unwrap();
        store.submit_request(public_submit_input()).unwrap();
        store
            .record_request_revision(RecordRequestRevisionInput {
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                new_head_oid: "head_2".to_string(),
                git_snapshot: None,
                event_id: "event_revision".to_string(),
                body: None,
                now_unix: 2,
            })
            .unwrap();
        let mut invalid_ref = public_submit_input();
        invalid_ref.id = "req_2".to_string();
        invalid_ref.event_id = "event_created_2".to_string();
        invalid_ref.stake_ledger_entry_id = Some("ledger_stake_2".to_string());
        let error = store.submit_request(invalid_ref).unwrap_err();
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
            .unwrap();
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

    fn public_submit_input() -> SubmitRequestInput {
        SubmitRequestInput {
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
            stake_credits: 10,
            stake_ledger_entry_id: Some("ledger_stake".to_string()),
            event_id: "event_created".to_string(),
            now_unix: 2,
        }
    }
}
