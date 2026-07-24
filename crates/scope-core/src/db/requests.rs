use super::{
    MetadataStore, acquire_aggregate_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    object_references::delete_object_reference,
    request_access::{
        authorize_start_request, ensure_user_exists, lock_request_repository, repo_by_id,
        request_policy_for_user,
    },
    request_change_block_rows::{change_blocks_for_request_ids, insert_change_block},
    request_discussion_rows::{insert_discussion, save_read_state},
    request_invitees::delete_request_invitees,
    request_rows::{
        credit_account_by_user_id, credit_ledger_entry_by_id, delete_request_rows,
        insert_credit_ledger_entry_row, insert_request_event_row, insert_request_row,
        latest_request_events, request_by_id, request_by_name, request_event_by_id,
        request_events_after_position, request_events_by_request_id, requests_by_repo_author,
        requests_by_repo_id, save_credit_account_row, save_request_row,
    },
};
use crate::{
    domain::requests::{
        CloseRequestInput, CloseRequestMutation, CreditAccountMutation, GrantUserCreditsInput,
        RecordRequestRevisionInput, RecordWorkingRequestUploadInput, Request, RequestEvent,
        RequestRevisionMutation, RequestTimelineMutation, StartRequestInput, StartRequestMutation,
        UpdateRequestDescriptionInput, WorkingRequestUploadMutation, close_request,
        grant_user_credits, record_request_revision, record_working_request_upload, start_request,
        update_request_description,
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

        let mut requests = requests_by_repo_author(&tx, &input.repo_id, &input.author_user_id)
            .await?
            .into_iter()
            .map(|request| (request.id.clone(), request))
            .collect::<BTreeMap<_, _>>();
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
        let (repo, request) = lock_request_repository(&tx, &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let mut input = input;
        input.actor_can_edit = request_policy_for_user(&tx, &repo, &request, &input.actor_user_id)
            .await?
            .branch_mutable;
        let mut requests = BTreeMap::from([(request.id.clone(), request)]);
        let mutation = record_working_request_upload(&mut requests, input)?;
        save_request_row(&tx, &mutation.request).await?;
        if !mutation.orphan_objects.is_empty() {
            queue_pending_source_blob_deletion_rows(&tx, mutation.orphan_objects.clone()).await?;
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
        let (repo, request) = lock_request_repository(&tx, &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let mut input = input;
        input.actor_can_edit = request_policy_for_user(&tx, &repo, &request, &input.actor_user_id)
            .await?
            .branch_mutable;
        let mut requests = BTreeMap::from([(request.id.clone(), request)]);
        let mut events = BTreeMap::new();
        if let Some(event) = request_event_by_id(&tx, &input.event_id).await? {
            events.insert(event.id.clone(), event);
        }
        let mutation = record_request_revision(&mut requests, &mut events, input)?;
        save_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        insert_change_block(&tx, &mutation.change_block).await?;
        insert_discussion(&tx, &mutation.discussion).await?;
        save_read_state(&tx, &mutation.read_state).await?;
        if !mutation.orphan_objects.is_empty() {
            queue_pending_source_blob_deletion_rows(&tx, mutation.orphan_objects.clone()).await?;
        }
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn update_request_description(
        &self,
        mut input: UpdateRequestDescriptionInput,
    ) -> Result<RequestTimelineMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        let (repo, request) = lock_request_repository(&tx, &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        input.actor_can_edit_description =
            request_policy_for_user(&tx, &repo, &request, &input.actor_user_id)
                .await?
                .permissions
                .can_edit_description;
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

    pub async fn close_request(
        &self,
        mut input: CloseRequestInput,
    ) -> Result<CloseRequestMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        let (_repo, request) = lock_request_repository(&tx, &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        input.actor_can_close = request.author_user_id == input.actor_user_id;
        let mut requests = BTreeMap::from([(request.id.clone(), request.clone())]);
        let mut events = request_events_by_request_id(&tx, &request.id)
            .await?
            .into_iter()
            .map(|event| (event.id.clone(), event))
            .collect::<BTreeMap<_, _>>();
        let mut change_blocks =
            change_blocks_for_request_ids(&tx, std::slice::from_ref(&request.id))
                .await?
                .into_iter()
                .map(|change_block| (change_block.id.clone(), change_block))
                .collect::<BTreeMap<_, _>>();
        let mutation = close_request(&mut requests, &mut events, &mut change_blocks, input)?;
        match &mutation {
            CloseRequestMutation::DeletedDraft {
                request,
                change_blocks,
                orphan_objects,
                ..
            } => {
                for change_block in change_blocks {
                    delete_object_reference(&tx, "request_change_block_snapshot", &change_block.id)
                        .await?;
                }
                delete_request_rows(&tx, &request.id).await?;
                if !orphan_objects.is_empty() {
                    queue_pending_source_blob_deletion_rows(&tx, orphan_objects.clone()).await?;
                }
            }
            CloseRequestMutation::Completed { request, event } => {
                save_request_row(&tx, request).await?;
                delete_request_invitees(&tx, &request.id).await?;
                insert_request_event_row(&tx, event).await?;
            }
        }
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }
}

#[cfg(test)]
mod tests;
