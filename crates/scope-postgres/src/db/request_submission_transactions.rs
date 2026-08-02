//! PostgreSQL transaction for one-way request submission.

use super::{
    RequestStore,
    request_access::{ensure_user_exists, lock_request_repository, request_policy_for_user},
    request_rows::{insert_request_event_row, request_event_by_id, save_request_row},
};
use sea_orm::{DatabaseTransaction, TransactionTrait};
use {
    crate::error::PostgresError,
    scope_domain::{
        requests::{Request, RequestLifecycleMutation, SubmitRequestInput, submit_request},
        store::StoredRepository,
    },
};

impl RequestStore {
    pub async fn submit_request(
        &self,
        mut input: SubmitRequestInput,
    ) -> Result<RequestLifecycleMutation, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_submission_context(&tx, &input.actor_user_id, &input.request_id).await?;
        input.actor_is_author = input.actor_user_id == request.author_user_id;
        input.actor_can_submit =
            request_policy_for_user(&tx, &repo, &request, &input.actor_user_id)
                .await?
                .permissions
                .can_submit;
        let mutation = submit_request(&request, input)?;
        persist_lifecycle_mutation(&tx, &mutation).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }
}

async fn lock_submission_context(
    tx: &DatabaseTransaction,
    actor_user_id: &str,
    request_id: &str,
) -> Result<(StoredRepository, Request), PostgresError> {
    let (repo, request) = lock_request_repository(tx, request_id).await?;
    ensure_user_exists(tx, actor_user_id).await?;
    Ok((repo, request))
}

pub(super) async fn persist_lifecycle_mutation(
    tx: &DatabaseTransaction,
    mutation: &RequestLifecycleMutation,
) -> Result<(), PostgresError> {
    for event in &mutation.events {
        if request_event_by_id(tx, &event.id).await?.is_some() {
            return Err(PostgresError::conflict("request event already exists"));
        }
    }
    save_request_row(tx, &mutation.request).await?;
    if mutation.request.is_terminal() {
        super::request_invitees::delete_request_invitees(tx, &mutation.request.id).await?;
    }
    for event in &mutation.events {
        insert_request_event_row(tx, event).await?;
    }
    Ok(())
}
