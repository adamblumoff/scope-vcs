//! Atomic Ready-review invalidation and request branch revision persistence.

use super::{
    MetadataStore, acquire_aggregate_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    request_access::{ensure_user_exists, repo_by_id, request_actor_can_edit},
    request_change_block_rows::insert_change_block,
    request_discussion_rows::{insert_discussion, save_read_state},
    request_rows::{
        credit_account_by_user_id, insert_credit_ledger_entry_row, insert_request_event_row,
        request_by_id, request_event_by_id, save_credit_account_row, save_request_row,
    },
};
use crate::{
    domain::requests::{
        RecordRequestRevisionInput, RequestActorRole, RequestEvent, RequestReviewExitReason,
        RequestState, ReturnRequestToWorkingInput, record_request_revision,
        return_request_to_working,
    },
    error::ApiError,
};
use sea_orm::TransactionTrait;
use std::collections::BTreeMap;

impl MetadataStore {
    pub async fn record_request_revision_with_review_invalidation(
        &self,
        mut input: RecordRequestRevisionInput,
    ) -> Result<crate::domain::requests::RequestRevisionMutation, ApiError> {
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        let observed = request_by_id(&tx, &input.request_id)
            .await?
            .ok_or_else(|| ApiError::not_found("request not found"))?;
        acquire_aggregate_lock(&tx, "repository", &observed.repo_id).await?;
        acquire_aggregate_lock(&tx, "request", &input.request_id).await?;
        let mut request = request_by_id(&tx, &input.request_id)
            .await?
            .filter(|request| request.repo_id == observed.repo_id)
            .ok_or_else(|| ApiError::not_found("request not found"))?;
        if request.author_role == RequestActorRole::Public
            && request.state == RequestState::ReadyForReview
        {
            acquire_aggregate_lock(&tx, "user-credit", &request.author_user_id).await?;
        }
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let repo = repo_by_id(&tx, &request.repo_id).await?;
        input.actor_can_edit = request_actor_can_edit(&repo, &request, &input.actor_user_id);

        if request.state == RequestState::ReadyForReview {
            let account = if request.author_role == RequestActorRole::Public {
                credit_account_by_user_id(&tx, &request.author_user_id).await?
            } else {
                None
            };
            let exit = return_request_to_working(
                &request,
                account.as_ref(),
                ReturnRequestToWorkingInput {
                    request_id: request.id.clone(),
                    actor_user_id: input.actor_user_id.clone(),
                    actor_is_author: input.actor_user_id == request.author_user_id,
                    actor_is_maintainer: repo.is_maintainer_user_id(&input.actor_user_id),
                    actor_can_mutate: input.actor_can_edit,
                    reason: RequestReviewExitReason::RevisionPushed,
                    event_id: format!("{}:review-invalidated", input.event_id),
                    now_unix: input.now_unix,
                },
            )?;
            save_request_row(&tx, &exit.request).await?;
            if let Some(account) = &exit.credit_account {
                save_credit_account_row(&tx, account).await?;
            }
            for entry in &exit.ledger_entries {
                insert_credit_ledger_entry_row(&tx, entry).await?;
            }
            for event in &exit.events {
                insert_request_event_row(&tx, event).await?;
            }
            request = exit.request;
        }

        let mut requests = BTreeMap::from([(request.id.clone(), request)]);
        let mut events = BTreeMap::<String, RequestEvent>::new();
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
}
