use super::{
    GeneratedIdSource, RequestStore, acquire_aggregate_lock,
    cleanup_queue::queue::queue_pending_source_blob_deletion_rows,
    object_references::delete_object_reference,
    request_access::{
        authorize_start_request, ensure_user_exists, lock_request_repository, repo_by_id,
        request_policy_for_user,
    },
    request_invitees::delete_request_invitees,
    request_revision_rows::{insert_revision, revisions_for_request_ids},
    request_rows::{
        delete_request_rows, insert_request_event_row, insert_request_row, latest_request_events,
        request_by_id, request_by_name, request_event_by_id, request_events_after_position,
        request_events_by_request_id, request_list_page, requests_by_repo_author,
        requests_by_repo_id, save_request_row,
    },
};
use sea_orm::TransactionTrait;
use std::{collections::BTreeMap, sync::Arc};
use {
    crate::error::PostgresError,
    scope_domain::requests::{
        CloseRequestInput, CloseRequestMutation, EditRequestIdentityInput,
        RecordRequestRevisionInput, RecordWorkingRequestUploadInput, Request, RequestEvent,
        RequestRevisionMutation, RequestTimelineMutation, StartRequestInput, StartRequestMutation,
        WorkingRequestUploadMutation, close_request, edit_request_identity,
        record_request_revision, record_working_request_upload, start_request,
    },
};

impl RequestStore {
    pub async fn request_list_page(
        &self,
        input: super::RequestListPageQuery<'_>,
    ) -> Result<Vec<super::RequestListRow>, PostgresError> {
        request_list_page(self.db.as_ref(), input).await
    }

    pub async fn request_by_id(&self, request_id: &str) -> Result<Option<Request>, PostgresError> {
        let request_id = request_id.to_string();
        let db = Arc::clone(&self.db);
        request_by_id(db.as_ref(), &request_id).await
    }

    pub async fn request_by_name(
        &self,
        repo_id: &str,
        request_name: &str,
    ) -> Result<Option<Request>, PostgresError> {
        let repo_id = repo_id.to_string();
        let request_name = request_name.to_string();
        let db = Arc::clone(&self.db);
        request_by_name(db.as_ref(), &repo_id, &request_name).await
    }

    pub async fn requests_by_repo_id(&self, repo_id: &str) -> Result<Vec<Request>, PostgresError> {
        let repo_id = repo_id.to_string();
        let db = Arc::clone(&self.db);
        requests_by_repo_id(db.as_ref(), &repo_id).await
    }

    pub async fn requests_by_repo_author(
        &self,
        repo_id: &str,
        author_user_id: &str,
    ) -> Result<Vec<Request>, PostgresError> {
        let repo_id = repo_id.to_string();
        let author_user_id = author_user_id.to_string();
        let db = Arc::clone(&self.db);
        requests_by_repo_author(db.as_ref(), &repo_id, &author_user_id).await
    }

    pub async fn request_events_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Vec<RequestEvent>, PostgresError> {
        let request_id = request_id.to_string();
        let db = Arc::clone(&self.db);
        request_events_by_request_id(db.as_ref(), &request_id).await
    }

    pub async fn request_events_after_position(
        &self,
        request_id: &str,
        after_position: u64,
        limit: u64,
    ) -> Result<Vec<RequestEvent>, PostgresError> {
        request_events_after_position(self.db.as_ref(), request_id, after_position, limit).await
    }

    pub async fn latest_request_events(
        &self,
        request_id: &str,
        limit: u64,
    ) -> Result<Vec<RequestEvent>, PostgresError> {
        latest_request_events(self.db.as_ref(), request_id, limit).await
    }

    pub async fn start_request(
        &self,
        input: StartRequestInput,
    ) -> Result<StartRequestMutation, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &input.repo_id).await?;
        acquire_aggregate_lock(&tx, "request", &input.id).await?;
        ensure_user_exists(&tx, &input.author_user_id).await?;
        let input = authorize_start_request(
            &repo_by_id(&tx, &input.repo_id, &input.author_user_id).await?,
            input,
        )?;

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
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn record_working_request_upload(
        &self,
        input: RecordWorkingRequestUploadInput,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<WorkingRequestUploadMutation, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &input.request_id, &input.actor_user_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let mut input = input;
        let now_unix = input.now_unix;
        input.actor_can_edit = request_policy_for_user(&tx, &repo, &request, &input.actor_user_id)
            .await?
            .branch_mutable;
        let mut requests = BTreeMap::from([(request.id.clone(), request)]);
        let mutation = record_working_request_upload(&mut requests, input)?;
        save_request_row(&tx, &mutation.request).await?;
        if !mutation.orphan_objects.is_empty() {
            queue_pending_source_blob_deletion_rows(
                &tx,
                mutation.orphan_objects.clone(),
                now_unix,
                generated_ids,
            )
            .await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn record_request_revision(
        &self,
        input: RecordRequestRevisionInput,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<RequestRevisionMutation, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &input.request_id, &input.actor_user_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let mut input = input;
        let now_unix = input.now_unix;
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
        insert_revision(&tx, &mutation.revision).await?;
        if !mutation.orphan_objects.is_empty() {
            queue_pending_source_blob_deletion_rows(
                &tx,
                mutation.orphan_objects.clone(),
                now_unix,
                generated_ids,
            )
            .await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn edit_request_identity(
        &self,
        mut input: EditRequestIdentityInput,
    ) -> Result<RequestTimelineMutation, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &input.request_id, &input.actor_user_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        input.actor_can_edit_identity =
            request_policy_for_user(&tx, &repo, &request, &input.actor_user_id)
                .await?
                .permissions
                .can_edit_identity;
        let mut requests = BTreeMap::from([(request.id.clone(), request)]);
        let mut events = BTreeMap::new();
        if let Some(event) = request_event_by_id(&tx, &input.event_id).await? {
            events.insert(event.id.clone(), event);
        }
        let mutation = edit_request_identity(&mut requests, &mut events, input)?;
        save_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn close_request(
        &self,
        mut input: CloseRequestInput,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<CloseRequestMutation, PostgresError> {
        let db = Arc::clone(&self.db);
        let now_unix = input.now_unix;
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &input.request_id, &input.actor_user_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        input.actor_is_author = request.author_user_id == input.actor_user_id;
        input.actor_is_maintainer = repo.access.is_maintainer();
        let mut requests = BTreeMap::from([(request.id.clone(), request.clone())]);
        let mut events = request_events_by_request_id(&tx, &request.id)
            .await?
            .into_iter()
            .map(|event| (event.id.clone(), event))
            .collect::<BTreeMap<_, _>>();
        let mut revisions = revisions_for_request_ids(&tx, std::slice::from_ref(&request.id))
            .await?
            .into_iter()
            .map(|revision| (revision.id.clone(), revision))
            .collect::<BTreeMap<_, _>>();
        let mutation = close_request(&mut requests, &mut events, &mut revisions, input)?;
        match &mutation {
            CloseRequestMutation::DeletedDraft {
                request,
                revisions,
                orphan_objects,
                ..
            } => {
                for revision in revisions {
                    delete_object_reference(&tx, "request_revision_snapshot", &revision.id).await?;
                }
                delete_request_rows(&tx, &request.id).await?;
                if !orphan_objects.is_empty() {
                    queue_pending_source_blob_deletion_rows(
                        &tx,
                        orphan_objects.clone(),
                        now_unix,
                        generated_ids,
                    )
                    .await?;
                }
            }
            CloseRequestMutation::Closed { request, event } => {
                save_request_row(&tx, request).await?;
                delete_request_invitees(&tx, &request.id).await?;
                insert_request_event_row(&tx, event).await?;
            }
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }
}

#[cfg(test)]
pub(super) mod tests;
