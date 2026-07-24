//! PostgreSQL transactions for request review lifecycle commands.
//!
//! Every command locks repository, request, then the public author's credit account. The
//! repository lock serializes Ready-cap admission. Exit paths use the same ordering but never
//! enforce the cap, so safety invalidation cannot be blocked.

use super::{
    MetadataStore, acquire_aggregate_lock,
    request_access::{ensure_user_exists, lock_request_repository, request_policy_for_user},
    request_invitees::delete_request_invitees,
    request_queue::next_ready_queue_version,
    request_rows::{
        credit_account_by_user_id, insert_credit_ledger_entry_row, insert_request_event_row,
        request_event_by_id, requests_by_repo_author, save_credit_account_row, save_request_row,
    },
};
use crate::{
    domain::{
        requests::{
            AssessRequestInput, MarkRequestReadyInput, Request, RequestActorRole,
            RequestReviewMutation, ReturnRequestToWorkingInput, SetRequestHoldInput,
            assess_request, mark_request_ready, return_request_to_working, set_request_hold,
        },
        store::{RepositoryActor, StoredRepository},
    },
    error::ApiError,
};
use sea_orm::{DatabaseTransaction, TransactionTrait};

impl MetadataStore {
    pub async fn mark_request_ready(
        &self,
        mut input: MarkRequestReadyInput,
    ) -> Result<RequestReviewMutation, ApiError> {
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        let (repo, request) =
            lock_review_context(&tx, &input.actor_user_id, &input.request_id, true).await?;
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
                        && candidate.state == crate::domain::requests::RequestState::ReadyForReview
                })
                .count()
        } else {
            0
        };
        input.ready_queue_version = next_ready_queue_version(&tx, &request.repo_id).await?;
        let account = load_credit_account(&tx, &request).await?;
        let mutation = mark_request_ready(&request, account.as_ref(), input)?;
        persist_review_mutation(&tx, &mutation).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn return_request_to_working(
        &self,
        mut input: ReturnRequestToWorkingInput,
    ) -> Result<RequestReviewMutation, ApiError> {
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        let (repo, request) =
            lock_review_context(&tx, &input.actor_user_id, &input.request_id, true).await?;
        input.actor_is_author = input.actor_user_id == request.author_user_id;
        input.actor_is_maintainer = is_maintainer(&repo, &input.actor_user_id);
        input.actor_can_mutate =
            request_policy_for_user(&tx, &repo, &request, &input.actor_user_id)
                .await?
                .branch_mutable;
        let account = load_credit_account(&tx, &request).await?;
        let mutation = return_request_to_working(&request, account.as_ref(), input)?;
        persist_review_mutation(&tx, &mutation).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn set_request_hold(
        &self,
        mut input: SetRequestHoldInput,
    ) -> Result<RequestReviewMutation, ApiError> {
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        let (repo, request) =
            lock_review_context(&tx, &input.actor_user_id, &input.request_id, false).await?;
        input.actor_is_maintainer = is_maintainer(&repo, &input.actor_user_id);
        let mutation = set_request_hold(&request, input)?;
        persist_review_mutation(&tx, &mutation).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn assess_request(
        &self,
        mut input: AssessRequestInput,
    ) -> Result<RequestReviewMutation, ApiError> {
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        let (repo, request) =
            lock_review_context(&tx, &input.actor_user_id, &input.request_id, true).await?;
        input.actor_is_maintainer = is_maintainer(&repo, &input.actor_user_id);
        let account = load_credit_account(&tx, &request).await?;
        let mutation = assess_request(&request, account.as_ref(), input)?;
        persist_review_mutation(&tx, &mutation).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }
}

async fn lock_review_context(
    tx: &DatabaseTransaction,
    actor_user_id: &str,
    request_id: &str,
    lock_user_credit: bool,
) -> Result<(StoredRepository, Request), ApiError> {
    let (repo, request) = lock_request_repository(tx, request_id).await?;
    if lock_user_credit && request.author_role == RequestActorRole::Public {
        acquire_aggregate_lock(tx, "user-credit", &request.author_user_id).await?;
    }
    ensure_user_exists(tx, actor_user_id).await?;
    Ok((repo, request))
}

async fn load_credit_account(
    tx: &DatabaseTransaction,
    request: &Request,
) -> Result<Option<crate::domain::requests::UserCreditAccount>, ApiError> {
    if request.author_role != RequestActorRole::Public {
        return Ok(None);
    }
    credit_account_by_user_id(tx, &request.author_user_id).await
}

pub(super) async fn persist_review_mutation(
    tx: &DatabaseTransaction,
    mutation: &RequestReviewMutation,
) -> Result<(), ApiError> {
    for event in &mutation.events {
        if request_event_by_id(tx, &event.id).await?.is_some() {
            return Err(ApiError::conflict("request command was already applied"));
        }
    }
    save_request_row(tx, &mutation.request).await?;
    if mutation.request.state == crate::domain::requests::RequestState::Completed {
        delete_request_invitees(tx, &mutation.request.id).await?;
    }
    if let Some(account) = &mutation.credit_account {
        save_credit_account_row(tx, account).await?;
    }
    for entry in &mutation.ledger_entries {
        insert_credit_ledger_entry_row(tx, entry).await?;
    }
    for event in &mutation.events {
        insert_request_event_row(tx, event).await?;
    }
    Ok(())
}

fn is_maintainer(repo: &StoredRepository, user_id: &str) -> bool {
    matches!(
        repo.access_for_user_id(user_id).actor,
        RepositoryActor::Owner | RepositoryActor::Member
    )
}
