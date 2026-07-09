#[cfg(any(test, feature = "memory-metadata"))]
use super::cleanup_queue::queue_pending_source_blob_deletions;
use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    request_access::{
        authorize_start_request, ensure_request_editor, ensure_request_maintainer,
        ensure_user_exists, repo_by_id, request_actor_can_edit,
    },
    request_rows::{
        credit_account_by_user_id, credit_ledger_entry_by_id, delete_request_rows,
        insert_credit_ledger_entry_row, insert_request_event_row, insert_request_row,
        request_by_id, request_by_ref, request_event_by_id, request_events_by_request_id,
        requests_by_repo_author, requests_by_repo_id, save_credit_account_row, save_request_row,
    },
    run_api_db_on,
};
use crate::{
    domain::{
        requests::{
            AddRequestEditorInput, CommentRequestInput, CreditAccountMutation, DeleteRequestInput,
            DeleteRequestMutation, GrantUserCreditsInput, MarkRequestNeedsResponseInput,
            MergeRequestInput, MergeRequestMutation, RecordRequestRevisionInput,
            RecordWorkingRequestUploadInput, RemoveRequestEditorInput, Request,
            RequestEditorMutation, RequestEvent, RequestRevisionMutation, RequestTimelineMutation,
            ResolveRequestInput, ResolveRequestMutation, RespondToRequestInput, StartRequestInput,
            StartRequestMutation, SubmitRequestInput, SubmitRequestMutation,
            WorkingRequestUploadMutation, add_request_editor, comment_request, delete_request,
            grant_user_credits, mark_request_needs_response, merge_request,
            record_request_revision, record_working_request_upload, remove_request_editor,
            resolve_request, respond_to_request, start_request, submit_request,
        },
        store::RepositoryActor,
    },
    error::ApiError,
};
use sea_orm::TransactionTrait;
use std::{collections::BTreeMap, sync::Arc};

impl MetadataStore {
    pub fn request_by_id(&self, request_id: &str) -> Result<Option<Request>, ApiError> {
        let request_id = request_id.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    request_by_id(db.as_ref(), &request_id).await
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => {
                self.read(move |catalog| Ok(catalog.requests.get(&request_id).cloned()))
            }
        }
    }

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

    pub fn requests_by_repo_id(&self, repo_id: &str) -> Result<Vec<Request>, ApiError> {
        let repo_id = repo_id.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    requests_by_repo_id(db.as_ref(), &repo_id).await
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.read(move |catalog| {
                let mut requests = catalog
                    .requests
                    .values()
                    .filter(|request| request.repo_id == repo_id)
                    .cloned()
                    .collect::<Vec<_>>();
                requests.sort_by(|left, right| {
                    left.created_at_unix
                        .cmp(&right.created_at_unix)
                        .then_with(|| left.id.cmp(&right.id))
                });
                Ok(requests)
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

    pub fn request_events_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Vec<RequestEvent>, ApiError> {
        let request_id = request_id.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    request_events_by_request_id(db.as_ref(), &request_id).await
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.read(move |catalog| {
                let mut events = catalog
                    .request_events
                    .values()
                    .filter(|event| event.request_id == request_id)
                    .cloned()
                    .collect::<Vec<_>>();
                events.sort_by(|left, right| {
                    left.created_at_unix
                        .cmp(&right.created_at_unix)
                        .then_with(|| left.id.cmp(&right.id))
                });
                Ok(events)
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

    pub fn start_request(
        &self,
        input: StartRequestInput,
    ) -> Result<StartRequestMutation, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    ensure_user_exists(&tx, &input.author_user_id).await?;
                    let input =
                        authorize_start_request(&repo_by_id(&tx, &input.repo_id).await?, input)?;

                    let mut requests = BTreeMap::new();
                    if let Some(request) = request_by_id(&tx, &input.id).await? {
                        requests.insert(request.id.clone(), request);
                    }
                    if let Some(request) = request_by_ref(&tx, &input.request_ref).await? {
                        requests.insert(request.id.clone(), request);
                    }

                    let mutation = start_request(&mut requests, input)?;
                    insert_request_row(&tx, &mutation.request).await?;
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
                let input = authorize_start_request(repo, input)?;
                start_request(&mut catalog.requests, input)
            }),
        }
    }

    pub fn record_working_request_upload(
        &self,
        input: RecordWorkingRequestUploadInput,
    ) -> Result<WorkingRequestUploadMutation, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    ensure_user_exists(&tx, &input.actor_user_id).await?;

                    let mut requests = BTreeMap::new();
                    if let Some(request) = request_by_id(&tx, &input.request_id).await? {
                        let repo = repo_by_id(&tx, &request.repo_id).await?;
                        let mut input = input;
                        input.actor_can_edit =
                            request_actor_can_edit(&repo, &request, &input.actor_user_id);
                        requests.insert(request.id.clone(), request);

                        let mutation = record_working_request_upload(&mut requests, input)?;
                        save_request_row(&tx, &mutation.request).await?;
                        if !mutation.source_blobs_to_delete.is_empty() {
                            queue_pending_source_blob_deletion_rows(
                                &tx,
                                mutation.source_blobs_to_delete.clone(),
                            )
                            .await?;
                        }
                        tx.commit().await.map_err(ApiError::internal)?;
                        return Ok(mutation);
                    }
                    Err(ApiError::not_found("request not found"))
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let request = catalog
                    .requests
                    .get(&input.request_id)
                    .ok_or_else(|| ApiError::not_found("request not found"))?;
                let repo = catalog
                    .repositories
                    .get(&request.repo_id)
                    .ok_or_else(|| ApiError::not_found("repo not found"))?;
                let mut input = input;
                input.actor_can_edit = request_actor_can_edit(repo, request, &input.actor_user_id);
                let mutation = record_working_request_upload(&mut catalog.requests, input)?;
                queue_pending_source_blob_deletions(
                    &mut catalog.pending_source_blob_deletions,
                    mutation.source_blobs_to_delete.clone(),
                );
                Ok(mutation)
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
                    ensure_user_exists(&tx, &input.actor_user_id).await?;

                    let mut requests = BTreeMap::new();
                    if let Some(request) = request_by_id(&tx, &input.request_id).await? {
                        requests.insert(request.id.clone(), request);
                    }
                    let mut events = BTreeMap::new();
                    if let Some(event) = request_event_by_id(&tx, &input.event_id).await? {
                        events.insert(event.id.clone(), event);
                    }
                    let mut accounts = BTreeMap::new();
                    if let Some(account) =
                        credit_account_by_user_id(&tx, &input.actor_user_id).await?
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
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                if !catalog.users.contains_key(&input.actor_user_id) {
                    return Err(ApiError::not_found("user not found"));
                }
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
                        let repo = repo_by_id(&tx, &request.repo_id).await?;
                        let mut input = input;
                        input.actor_can_edit =
                            request_actor_can_edit(&repo, &request, &input.actor_user_id);
                        requests.insert(request.id.clone(), request);
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
                        return Ok(mutation);
                    }
                    Err(ApiError::not_found("request not found"))
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let request = catalog
                    .requests
                    .get(&input.request_id)
                    .ok_or_else(|| ApiError::not_found("request not found"))?;
                let repo = catalog
                    .repositories
                    .get(&request.repo_id)
                    .ok_or_else(|| ApiError::not_found("repo not found"))?;
                let mut input = input;
                input.actor_can_edit = request_actor_can_edit(repo, request, &input.actor_user_id);
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

    pub fn comment_request(
        &self,
        input: CommentRequestInput,
    ) -> Result<RequestTimelineMutation, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    ensure_user_exists(&tx, &input.actor_user_id).await?;

                    let mut requests = BTreeMap::new();
                    let request = request_by_id(&tx, &input.request_id)
                        .await?
                        .ok_or_else(|| ApiError::not_found("request not found"))?;
                    let repo = repo_by_id(&tx, &request.repo_id).await?;
                    ensure_request_editor(&repo, &request, &input.actor_user_id)?;
                    requests.insert(request.id.clone(), request);
                    let mut events = BTreeMap::new();
                    if let Some(event) = request_event_by_id(&tx, &input.event_id).await? {
                        events.insert(event.id.clone(), event);
                    }

                    let mutation = comment_request(&mut requests, &mut events, input)?;
                    save_request_row(&tx, &mutation.request).await?;
                    insert_request_event_row(&tx, &mutation.event).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(mutation)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                if !catalog.users.contains_key(&input.actor_user_id) {
                    return Err(ApiError::not_found("user not found"));
                }
                let request = catalog
                    .requests
                    .get(&input.request_id)
                    .ok_or_else(|| ApiError::not_found("request not found"))?;
                let repo = catalog
                    .repositories
                    .get(&request.repo_id)
                    .ok_or_else(|| ApiError::not_found("repo not found"))?;
                ensure_request_editor(repo, request, &input.actor_user_id)?;
                comment_request(&mut catalog.requests, &mut catalog.request_events, input)
            }),
        }
    }

    pub fn mark_request_needs_response(
        &self,
        input: MarkRequestNeedsResponseInput,
    ) -> Result<RequestTimelineMutation, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
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
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                if !catalog.users.contains_key(&input.actor_user_id) {
                    return Err(ApiError::not_found("user not found"));
                }
                let request = catalog
                    .requests
                    .get(&input.request_id)
                    .ok_or_else(|| ApiError::not_found("request not found"))?;
                let repo = catalog
                    .repositories
                    .get(&request.repo_id)
                    .ok_or_else(|| ApiError::not_found("repo not found"))?;
                ensure_request_maintainer(repo, &input.actor_user_id)?;
                mark_request_needs_response(
                    &mut catalog.requests,
                    &mut catalog.request_events,
                    input,
                )
            }),
        }
    }

    pub fn respond_to_request(
        &self,
        input: RespondToRequestInput,
    ) -> Result<RequestTimelineMutation, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
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
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                if !catalog.users.contains_key(&input.actor_user_id) {
                    return Err(ApiError::not_found("user not found"));
                }
                respond_to_request(&mut catalog.requests, &mut catalog.request_events, input)
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

    pub fn add_request_editor(
        &self,
        input: AddRequestEditorInput,
    ) -> Result<RequestEditorMutation, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    ensure_user_exists(&tx, &input.actor_user_id).await?;
                    ensure_user_exists(&tx, &input.editor_user_id).await?;
                    let request = request_by_id(&tx, &input.request_id)
                        .await?
                        .ok_or_else(|| ApiError::not_found("request not found"))?;
                    let repo = repo_by_id(&tx, &request.repo_id).await?;
                    ensure_request_maintainer(&repo, &input.actor_user_id)?;
                    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
                    let mutation = add_request_editor(&mut requests, input)?;
                    save_request_row(&tx, &mutation.request).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(mutation)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                if !catalog.users.contains_key(&input.actor_user_id)
                    || !catalog.users.contains_key(&input.editor_user_id)
                {
                    return Err(ApiError::not_found("user not found"));
                }
                let request = catalog
                    .requests
                    .get(&input.request_id)
                    .ok_or_else(|| ApiError::not_found("request not found"))?;
                let repo = catalog
                    .repositories
                    .get(&request.repo_id)
                    .ok_or_else(|| ApiError::not_found("repo not found"))?;
                ensure_request_maintainer(repo, &input.actor_user_id)?;
                add_request_editor(&mut catalog.requests, input)
            }),
        }
    }

    pub fn remove_request_editor(
        &self,
        input: RemoveRequestEditorInput,
    ) -> Result<RequestEditorMutation, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    ensure_user_exists(&tx, &input.actor_user_id).await?;
                    let request = request_by_id(&tx, &input.request_id)
                        .await?
                        .ok_or_else(|| ApiError::not_found("request not found"))?;
                    let repo = repo_by_id(&tx, &request.repo_id).await?;
                    ensure_request_maintainer(&repo, &input.actor_user_id)?;
                    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
                    let mutation = remove_request_editor(&mut requests, input)?;
                    save_request_row(&tx, &mutation.request).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(mutation)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                if !catalog.users.contains_key(&input.actor_user_id) {
                    return Err(ApiError::not_found("user not found"));
                }
                let request = catalog
                    .requests
                    .get(&input.request_id)
                    .ok_or_else(|| ApiError::not_found("request not found"))?;
                let repo = catalog
                    .repositories
                    .get(&request.repo_id)
                    .ok_or_else(|| ApiError::not_found("repo not found"))?;
                ensure_request_maintainer(repo, &input.actor_user_id)?;
                remove_request_editor(&mut catalog.requests, input)
            }),
        }
    }

    pub fn delete_request(
        &self,
        input: DeleteRequestInput,
    ) -> Result<DeleteRequestMutation, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    ensure_user_exists(&tx, &input.actor_user_id).await?;
                    let request = request_by_id(&tx, &input.request_id)
                        .await?
                        .ok_or_else(|| ApiError::not_found("request not found"))?;
                    let repo = repo_by_id(&tx, &request.repo_id).await?;
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
                    if let Some(account) =
                        credit_account_by_user_id(&tx, &request.author_user_id).await?
                    {
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
                            source_blobs_to_delete,
                            ..
                        } => {
                            delete_request_rows(&tx, &request.id).await?;
                            if !source_blobs_to_delete.is_empty() {
                                queue_pending_source_blob_deletion_rows(
                                    &tx,
                                    source_blobs_to_delete.clone(),
                                )
                                .await?;
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
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let request = catalog
                    .requests
                    .get(&input.request_id)
                    .ok_or_else(|| ApiError::not_found("request not found"))?
                    .clone();
                if !catalog.users.contains_key(&input.actor_user_id) {
                    return Err(ApiError::not_found("user not found"));
                }
                let repo = catalog
                    .repositories
                    .get(&request.repo_id)
                    .ok_or_else(|| ApiError::not_found("repo not found"))?;
                let mut input = input;
                input.actor_can_delete = request.author_user_id == input.actor_user_id
                    || matches!(
                        repo.access_for_user_id(&input.actor_user_id).actor,
                        RepositoryActor::Owner | RepositoryActor::Member
                    );
                let mutation = delete_request(
                    &mut catalog.requests,
                    &mut catalog.request_events,
                    &mut catalog.user_credit_accounts,
                    &mut catalog.credit_ledger_entries,
                    input,
                )?;
                if let DeleteRequestMutation::DeletedWorking {
                    source_blobs_to_delete,
                    ..
                } = &mutation
                {
                    queue_pending_source_blob_deletions(
                        &mut catalog.pending_source_blob_deletions,
                        source_blobs_to_delete.clone(),
                    );
                }
                Ok(mutation)
            }),
        }
    }
}

#[cfg(test)]
mod tests;
