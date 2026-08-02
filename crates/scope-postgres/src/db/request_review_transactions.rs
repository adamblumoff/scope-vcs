//! PostgreSQL transactions for request review lifecycle commands.
//!
//! Every command locks repository then request. The repository lock serializes Ready-cap
//! admission.

use super::{
    RequestStore,
    request_access::{ensure_user_exists, lock_request_repository, request_policy_for_user},
    request_invitees::delete_request_invitees,
    request_rows::{
        insert_request_event_row, request_event_by_id, requests_by_repo_author, save_request_row,
    },
};
use sea_orm::{DatabaseTransaction, TransactionTrait};
use {
    crate::error::PostgresError,
    scope_domain::{
        requests::{
            MarkRequestReadyInput, Request, RequestActorRole, RequestReviewMutation,
            ReturnRequestToWorkingInput, mark_request_ready, return_request_to_working,
        },
        store::StoredRepository,
    },
};

impl RequestStore {
    pub async fn mark_request_ready(
        &self,
        mut input: MarkRequestReadyInput,
    ) -> Result<RequestReviewMutation, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_review_context(&tx, &input.actor_user_id, &input.request_id).await?;
        input.actor_is_author = input.actor_user_id == request.author_user_id;
        input.actor_can_mutate =
            request_policy_for_user(&tx, &repo, &request, &input.actor_user_id)
                .await?
                .permissions
                .can_mark_ready;
        input.public_ready_count = if request.author_role == RequestActorRole::Public {
            requests_by_repo_author(&tx, &request.repo_id, &request.author_user_id)
                .await?
                .into_iter()
                .filter(|candidate| {
                    candidate.author_role == RequestActorRole::Public
                        && candidate.state == scope_domain::requests::RequestState::ReadyForReview
                })
                .count()
        } else {
            0
        };
        let mutation = mark_request_ready(&request, input)?;
        persist_review_mutation(&tx, &mutation).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn return_request_to_working(
        &self,
        mut input: ReturnRequestToWorkingInput,
    ) -> Result<RequestReviewMutation, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_review_context(&tx, &input.actor_user_id, &input.request_id).await?;
        input.actor_is_author = input.actor_user_id == request.author_user_id;
        input.actor_can_mutate =
            request_policy_for_user(&tx, &repo, &request, &input.actor_user_id)
                .await?
                .branch_mutable;
        let mutation = return_request_to_working(&request, input)?;
        persist_review_mutation(&tx, &mutation).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }
}

async fn lock_review_context(
    tx: &DatabaseTransaction,
    actor_user_id: &str,
    request_id: &str,
) -> Result<(StoredRepository, Request), PostgresError> {
    let (repo, request) = lock_request_repository(tx, request_id).await?;
    ensure_user_exists(tx, actor_user_id).await?;
    Ok((repo, request))
}

pub(super) async fn persist_review_mutation(
    tx: &DatabaseTransaction,
    mutation: &RequestReviewMutation,
) -> Result<(), PostgresError> {
    for event in &mutation.events {
        if request_event_by_id(tx, &event.id).await?.is_some() {
            return Err(PostgresError::conflict(
                "request command was already applied",
            ));
        }
    }
    save_request_row(tx, &mutation.request).await?;
    if mutation.request.state == scope_domain::requests::RequestState::Completed {
        delete_request_invitees(tx, &mutation.request.id).await?;
    }
    for event in &mutation.events {
        insert_request_event_row(tx, event).await?;
    }
    Ok(())
}
