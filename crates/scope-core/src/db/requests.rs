use super::{
    MetadataStore, acquire_aggregate_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    request_access::{
        authorize_start_request, ensure_request_maintainer, ensure_user_exists, repo_by_id,
        request_actor_can_edit,
    },
    request_rows::{
        credit_account_by_user_id, credit_ledger_entry_by_id, delete_request_rows,
        insert_credit_ledger_entry_row, insert_request_event_row, insert_request_row,
        latest_request_events, request_by_id, request_by_name, request_event_by_id,
        request_events_after_position, request_events_by_request_id, requests_by_repo_author,
        requests_by_repo_id, save_credit_account_row, save_request_row,
    },
};
use crate::{
    domain::{
        requests::{
            CreditAccountMutation, DeleteRequestInput, DeleteRequestMutation,
            GrantUserCreditsInput, MarkRequestNeedsResponseInput, MergeRequestInput,
            MergeRequestMutation, RecordRequestRevisionInput, RecordWorkingRequestUploadInput,
            Request, RequestEvent, RequestRevisionMutation, RequestTimelineMutation,
            ResolveRequestInput, ResolveRequestMutation, RespondToRequestInput, StartRequestInput,
            StartRequestMutation, SubmitRequestInput, SubmitRequestMutation,
            UpdateRequestDescriptionInput, WorkingRequestUploadMutation, delete_request,
            grant_user_credits, mark_request_needs_response, merge_request,
            record_request_revision, record_working_request_upload, resolve_request,
            respond_to_request, start_request, submit_request, update_request_description,
        },
        store::{RepositoryActor, StoredRepository},
    },
    error::ApiError,
};
use sea_orm::TransactionTrait;
use std::{collections::BTreeMap, sync::Arc};

impl MetadataStore {
    pub async fn request_by_id(&self, request_id: &str) -> Result<Option<Request>, ApiError> {
        let request_id = request_id.to_string();
        let db = Arc::clone(&self.db);
        request_by_id(db.as_ref(), &request_id).await
    }

    pub async fn request_by_name(
        &self,
        repo_id: &str,
        request_name: &str,
    ) -> Result<Option<Request>, ApiError> {
        let repo_id = repo_id.to_string();
        let request_name = request_name.to_string();
        let db = Arc::clone(&self.db);
        request_by_name(db.as_ref(), &repo_id, &request_name).await
    }

    pub async fn requests_by_repo_id(&self, repo_id: &str) -> Result<Vec<Request>, ApiError> {
        let repo_id = repo_id.to_string();
        let db = Arc::clone(&self.db);
        requests_by_repo_id(db.as_ref(), &repo_id).await
    }

    pub async fn requests_by_repo_author(
        &self,
        repo_id: &str,
        author_user_id: &str,
    ) -> Result<Vec<Request>, ApiError> {
        let repo_id = repo_id.to_string();
        let author_user_id = author_user_id.to_string();
        let db = Arc::clone(&self.db);
        requests_by_repo_author(db.as_ref(), &repo_id, &author_user_id).await
    }

    pub async fn request_events_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Vec<RequestEvent>, ApiError> {
        let request_id = request_id.to_string();
        let db = Arc::clone(&self.db);
        request_events_by_request_id(db.as_ref(), &request_id).await
    }

    pub async fn request_events_after_position(
        &self,
        request_id: &str,
        after_position: u64,
        limit: u64,
    ) -> Result<Vec<RequestEvent>, ApiError> {
        request_events_after_position(self.db.as_ref(), request_id, after_position, limit).await
    }

    pub async fn latest_request_events(
        &self,
        request_id: &str,
        limit: u64,
    ) -> Result<Vec<RequestEvent>, ApiError> {
        latest_request_events(self.db.as_ref(), request_id, limit).await
    }

    pub async fn grant_user_credits(
        &self,
        input: GrantUserCreditsInput,
    ) -> Result<CreditAccountMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "user-credit", &input.user_id).await?;
        ensure_user_exists(&tx, &input.user_id).await?;

        let mut accounts = BTreeMap::new();
        if let Some(account) = credit_account_by_user_id(&tx, &input.user_id).await? {
            accounts.insert(account.user_id.clone(), account);
        }
        let mut ledger_entries = BTreeMap::new();
        if let Some(entry) = credit_ledger_entry_by_id(&tx, &input.ledger_entry_id).await? {
            ledger_entries.insert(entry.id.clone(), entry);
        }

        let mutation = grant_user_credits(&mut accounts, &mut ledger_entries, input)?;
        save_credit_account_row(&tx, &mutation.account).await?;
        insert_credit_ledger_entry_row(&tx, &mutation.ledger_entry).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn start_request(
        &self,
        input: StartRequestInput,
    ) -> Result<StartRequestMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &input.repo_id).await?;
        acquire_aggregate_lock(&tx, "request", &input.id).await?;
        ensure_user_exists(&tx, &input.author_user_id).await?;
        let input = authorize_start_request(&repo_by_id(&tx, &input.repo_id).await?, input)?;

        let mut requests = BTreeMap::new();
        if let Some(request) = request_by_id(&tx, &input.id).await? {
            requests.insert(request.id.clone(), request);
        }
        if let Some(request) = request_by_name(&tx, &input.repo_id, &input.name).await? {
            requests.insert(request.id.clone(), request);
        }

        let mutation = start_request(&mut requests, input)?;
        insert_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn record_working_request_upload(
        &self,
        input: RecordWorkingRequestUploadInput,
    ) -> Result<WorkingRequestUploadMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "request", &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;

        let mut requests = BTreeMap::new();
        if let Some(request) = request_by_id(&tx, &input.request_id).await? {
            let repo = repo_by_id(&tx, &request.repo_id).await?;
            let mut input = input;
            input.actor_can_edit = request_actor_can_edit(&repo, &request, &input.actor_user_id);
            requests.insert(request.id.clone(), request);

            let mutation = record_working_request_upload(&mut requests, input)?;
            save_request_row(&tx, &mutation.request).await?;
            if !mutation.orphan_objects.is_empty() {
                queue_pending_source_blob_deletion_rows(&tx, mutation.orphan_objects.clone())
                    .await?;
            }
            tx.commit().await.map_err(ApiError::internal)?;
            return Ok(mutation);
        }
        Err(ApiError::not_found("request not found"))
    }

    pub async fn submit_request(
        &self,
        input: SubmitRequestInput,
    ) -> Result<SubmitRequestMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        let (_, request) = lock_request_repository(&tx, &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;

        let mut requests = BTreeMap::from([(request.id.clone(), request)]);
        let mut events = BTreeMap::new();
        if let Some(event) = request_event_by_id(&tx, &input.event_id).await? {
            events.insert(event.id.clone(), event);
        }
        acquire_aggregate_lock(&tx, "user-credit", &input.actor_user_id).await?;
        let mut accounts = BTreeMap::new();
        if let Some(account) = credit_account_by_user_id(&tx, &input.actor_user_id).await? {
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
        save_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        if let Some(account) = &mutation.account {
            save_credit_account_row(&tx, account).await?;
        }
        if let Some(entry) = &mutation.ledger_entry {
            insert_credit_ledger_entry_row(&tx, entry).await?;
        }
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn record_request_revision(
        &self,
        input: RecordRequestRevisionInput,
    ) -> Result<RequestRevisionMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "request", &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;

        let mut requests = BTreeMap::new();
        if let Some(request) = request_by_id(&tx, &input.request_id).await? {
            let repo = repo_by_id(&tx, &request.repo_id).await?;
            let mut input = input;
            input.actor_can_edit = request_actor_can_edit(&repo, &request, &input.actor_user_id);
            requests.insert(request.id.clone(), request);
            let mut events = BTreeMap::new();
            if let Some(event) = request_event_by_id(&tx, &input.event_id).await? {
                events.insert(event.id.clone(), event);
            }

            let mutation = record_request_revision(&mut requests, &mut events, input)?;
            save_request_row(&tx, &mutation.request).await?;
            insert_request_event_row(&tx, &mutation.event).await?;
            if !mutation.orphan_objects.is_empty() {
                queue_pending_source_blob_deletion_rows(&tx, mutation.orphan_objects.clone())
                    .await?;
            }
            tx.commit().await.map_err(ApiError::internal)?;
            return Ok(mutation);
        }
        Err(ApiError::not_found("request not found"))
    }

    pub async fn mark_request_needs_response(
        &self,
        input: MarkRequestNeedsResponseInput,
    ) -> Result<RequestTimelineMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "request", &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;

        let mut requests = BTreeMap::new();
        let request = request_by_id(&tx, &input.request_id)
            .await?
            .ok_or_else(|| ApiError::not_found("request not found"))?;
        let repo = repo_by_id(&tx, &request.repo_id).await?;
        ensure_request_maintainer(&repo, &input.actor_user_id)?;
        requests.insert(request.id.clone(), request);
        let mut events = BTreeMap::new();
        if let Some(event) = request_event_by_id(&tx, &input.event_id).await? {
            events.insert(event.id.clone(), event);
        }

        let mutation = mark_request_needs_response(&mut requests, &mut events, input)?;
        save_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn update_request_description(
        &self,
        mut input: UpdateRequestDescriptionInput,
    ) -> Result<RequestTimelineMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "request", &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let request = request_by_id(&tx, &input.request_id)
            .await?
            .ok_or_else(|| ApiError::not_found("request not found"))?;
        let repo = repo_by_id(&tx, &request.repo_id).await?;
        input.actor_can_edit_description = request.author_user_id == input.actor_user_id
            || repo.is_maintainer_user_id(&input.actor_user_id);
        let mut requests = BTreeMap::from([(request.id.clone(), request)]);
        let mut events = BTreeMap::new();
        if let Some(event) = request_event_by_id(&tx, &input.event_id).await? {
            events.insert(event.id.clone(), event);
        }
        let mutation = update_request_description(&mut requests, &mut events, input)?;
        save_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn respond_to_request(
        &self,
        input: RespondToRequestInput,
    ) -> Result<RequestTimelineMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "request", &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;

        let mut requests = BTreeMap::new();
        let request = request_by_id(&tx, &input.request_id)
            .await?
            .ok_or_else(|| ApiError::not_found("request not found"))?;
        requests.insert(request.id.clone(), request);
        let mut events = BTreeMap::new();
        if let Some(event) = request_event_by_id(&tx, &input.event_id).await? {
            events.insert(event.id.clone(), event);
        }

        let mutation = respond_to_request(&mut requests, &mut events, input)?;
        save_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn resolve_request(
        &self,
        input: ResolveRequestInput,
    ) -> Result<ResolveRequestMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        let (repo, request) = lock_request_repository(&tx, &input.request_id).await?;
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
    }

    pub async fn merge_request(
        &self,
        input: MergeRequestInput,
    ) -> Result<MergeRequestMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        let (repo, request) = lock_request_repository(&tx, &input.request_id).await?;
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
    }

    pub async fn delete_request(
        &self,
        input: DeleteRequestInput,
    ) -> Result<DeleteRequestMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        let (repo, request) = lock_request_repository(&tx, &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        acquire_aggregate_lock(&tx, "user-credit", &request.author_user_id).await?;
        let mut input = input;
        input.actor_can_delete = request.author_user_id == input.actor_user_id
            || matches!(
                repo.access_for_user_id(&input.actor_user_id).actor,
                RepositoryActor::Owner | RepositoryActor::Member
            );
        let mut requests = BTreeMap::from([(request.id.clone(), request.clone())]);
        let mut events = request_events_by_request_id(&tx, &request.id)
            .await?
            .into_iter()
            .map(|event| (event.id.clone(), event))
            .collect::<BTreeMap<_, _>>();
        let mut accounts = BTreeMap::new();
        if let Some(account) = credit_account_by_user_id(&tx, &request.author_user_id).await? {
            accounts.insert(account.user_id.clone(), account);
        }
        let mut ledger_entries = BTreeMap::new();
        if let Some(entry_id) = input.refund_ledger_entry_id.as_deref()
            && let Some(entry) = credit_ledger_entry_by_id(&tx, entry_id).await?
        {
            ledger_entries.insert(entry.id.clone(), entry);
        }
        let mutation = delete_request(
            &mut requests,
            &mut events,
            &mut accounts,
            &mut ledger_entries,
            input,
        )?;
        match &mutation {
            DeleteRequestMutation::DeletedWorking {
                request,
                orphan_objects,
                ..
            } => {
                delete_request_rows(&tx, &request.id).await?;
                if !orphan_objects.is_empty() {
                    queue_pending_source_blob_deletion_rows(&tx, orphan_objects.clone()).await?;
                }
            }
            DeleteRequestMutation::Withdrawn {
                request,
                event,
                account,
                ledger_entry,
            } => {
                save_request_row(&tx, request).await?;
                insert_request_event_row(&tx, event).await?;
                if let Some(account) = account {
                    save_credit_account_row(&tx, account).await?;
                }
                if let Some(entry) = ledger_entry {
                    insert_credit_ledger_entry_row(&tx, entry).await?;
                }
            }
        }
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }
}

async fn lock_request_repository<C>(
    conn: &C,
    request_id: &str,
) -> Result<(StoredRepository, Request), ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let observed = request_by_id(conn, request_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    acquire_aggregate_lock(conn, "repository", &observed.repo_id).await?;
    acquire_aggregate_lock(conn, "request", request_id).await?;
    let request = request_by_id(conn, request_id)
        .await?
        .filter(|request| request.repo_id == observed.repo_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    let repo = repo_by_id(conn, &request.repo_id).await?;
    Ok((repo, request))
}

#[cfg(test)]
mod tests;
