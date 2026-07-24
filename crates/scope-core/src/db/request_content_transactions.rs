//! Atomic Ready-review invalidation for request content edits.

use super::{
    MetadataStore, acquire_aggregate_lock,
    request_access::{ensure_user_exists, lock_request_repository, request_policy_for_user},
    request_rows::{
        credit_account_by_user_id, insert_credit_ledger_entry_row, insert_request_event_row,
        request_event_by_id, save_credit_account_row, save_request_row,
    },
};
use crate::{
    domain::requests::{
        Request, RequestActorRole, RequestEvent, RequestReviewExitReason, RequestState,
        RequestTimelineMutation, ReturnRequestToWorkingInput, UpdateRequestDescriptionInput,
        return_request_to_working, update_request_description,
    },
    error::ApiError,
};
use sea_orm::TransactionTrait;
use std::collections::BTreeMap;

impl MetadataStore {
    pub async fn update_request_description_with_review_invalidation(
        &self,
        mut input: UpdateRequestDescriptionInput,
    ) -> Result<RequestTimelineMutation, ApiError> {
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        let (repo, mut request) = lock_request_repository(&tx, &input.request_id).await?;
        if request.author_role == RequestActorRole::Public
            && request.state == RequestState::ReadyForReview
        {
            acquire_aggregate_lock(&tx, "user-credit", &request.author_user_id).await?;
        }
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let actor_is_author = input.actor_user_id == request.author_user_id;
        let actor_is_maintainer = repo.is_maintainer_user_id(&input.actor_user_id);
        input.actor_can_edit_description =
            request_policy_for_user(&tx, &repo, &request, &input.actor_user_id)
                .await?
                .permissions
                .can_edit_description;

        if request.state == RequestState::ReadyForReview {
            let account = load_account(&tx, &request).await?;
            let exit = return_request_to_working(
                &request,
                account.as_ref(),
                ReturnRequestToWorkingInput {
                    request_id: request.id.clone(),
                    actor_user_id: input.actor_user_id.clone(),
                    actor_is_author,
                    actor_is_maintainer,
                    actor_can_mutate: input.actor_can_edit_description,
                    reason: RequestReviewExitReason::ContentEdited,
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
        let mutation = update_request_description(&mut requests, &mut events, input)?;
        save_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }
}

async fn load_account(
    tx: &sea_orm::DatabaseTransaction,
    request: &Request,
) -> Result<Option<crate::domain::requests::UserCreditAccount>, ApiError> {
    if request.author_role == RequestActorRole::Public {
        credit_account_by_user_id(tx, &request.author_user_id).await
    } else {
        Ok(None)
    }
}
