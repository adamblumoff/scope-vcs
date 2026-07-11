use crate::{
    auth::scope::{optional_scope_user, principal_for_scope_user, require_scope_user},
    config::DEFAULT_GIT_BRANCH,
    domain::{
        projection::{ProjectionViewKey, project_graph},
        requests::{
            CommentRequestInput, DeleteRequestInput, DeleteRequestMutation,
            MarkRequestNeedsResponseInput, MergeRequestInput, Request, RequestActorRole,
            RequestAudience, RequestDisposition, RequestState, ResolveRequestInput,
            RespondToRequestInput, StartRequestInput, SubmitRequestInput, canonical_request_ref,
            request_actor_role, request_mergeability, request_permissions,
            request_visible_to_access, settlement_for,
        },
        store::{RepositoryAccess, RepositoryActor, StoredRepository},
    },
    error::ApiError,
    git::{
        import::{apply_request_merge_update, git_refs},
        request_refs::delete_request_ref_from_store,
        storage::cached_raw_git_snapshot_repo,
        upload::projection_bare_repo_for_state,
    },
    http::{request_merges::clean_merge_update, responses::*},
    persistence::unix_now,
    state::{
        AppState, best_effort_drain_pending_source_blob_deletions, ensure_repo_read, find_repo,
        repo_config_fingerprint,
    },
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};

const REQUEST_SUMMARY_REFRESH_VERSION: u64 = 0;

pub(crate) async fn list_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RequestListResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
    let requests = state
        .metadata
        .requests_by_repo_id(&repo.record.id)
        .await?
        .into_iter()
        .filter(|request| request_visible_to_access(request, access))
        .map(|request| {
            request_response(
                request,
                access,
                current_main_oid.clone(),
                viewer_user_id.as_deref(),
            )
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(Json(RequestListResponse { requests }))
}

pub(crate) async fn get_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestDetailResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
    let events = state
        .metadata
        .request_events_by_request_id(&request.id)
        .await?
        .into_iter()
        .map(request_event_response)
        .collect::<Result<Vec<_>, ApiError>>()?;
    let request = request_response(request, access, current_main_oid, viewer_user_id.as_deref())?;

    Ok(Json(RequestDetailResponse { request, events }))
}

pub(crate) async fn delete_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestDeleteResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    if !request_permissions(&request, access, Some(&user.id)).can_delete {
        return Err(ApiError::forbidden("request delete access required"));
    }
    let refund_ledger_entry_id = (request.stake_credits > 0)
        .then(|| random_id("ledger_request_withdraw_refund"))
        .transpose()?;
    let request_ref = canonical_request_ref(&request.name);
    let mutation = state
        .metadata
        .delete_request(DeleteRequestInput {
            request_id: request.id,
            actor_user_id: user.id.clone(),
            actor_can_delete: false,
            event_id: random_id("event_request_withdrawn")?,
            refund_ledger_entry_id,
            now_unix: unix_now()?,
        })
        .await?;
    match mutation {
        DeleteRequestMutation::DeletedWorking { .. } => {
            delete_request_ref_from_store(&state, &owner, &repo_name, &request_ref)?;
            best_effort_drain_pending_source_blob_deletions(&state).await;
            publish_request_summary_refresh(&state, &repo, "request-deleted").await;
            Ok(Json(RequestDeleteResponse {
                deleted: true,
                request: None,
            }))
        }
        DeleteRequestMutation::Withdrawn { request, .. } => {
            let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
            let request = request_response(*request, access, current_main_oid, Some(&user.id))?;
            publish_request_summary_refresh(&state, &repo, "request-withdrawn").await;
            Ok(Json(RequestDeleteResponse {
                deleted: false,
                request: Some(request),
            }))
        }
    }
}

pub(crate) async fn start_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<StartRequestRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = find_repo(&state, &owner, &repo_name).await?;
    let principal = principal_for_scope_user(&repo, Some(&user));
    ensure_repo_read(&state, &repo, &principal)?;
    let access = repo.access_for_principal(&principal);
    if access.actor == RepositoryActor::Public && input.audience != RequestAudience::Public {
        return Err(ApiError::forbidden(
            "public contributors can only create public requests",
        ));
    }
    let base_main_oid = current_main_oid_for_audience(&state, &repo, input.audience)?
        .ok_or_else(|| ApiError::conflict("repo has no main branch to base a request on"))?;
    let request_id = random_id("req")?;
    let now_unix = unix_now()?;
    let mutation = state
        .metadata
        .start_request(StartRequestInput {
            id: request_id.clone(),
            repo_id: repo.record.id.clone(),
            name: input.name,
            author_user_id: user.id.clone(),
            title: input.title,
            author_role: request_actor_role(access),
            audience: input.audience,
            base_main_oid,
            now_unix,
        })
        .await?;
    let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
    let request = request_response(mutation.request, access, current_main_oid, Some(&user.id))?;
    publish_request_summary_refresh(&state, &repo, "request-started").await;
    Ok(Json(RequestMutationResponse { request }))
}

pub(crate) async fn submit_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<SubmitRequestRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = find_repo(&state, &owner, &repo_name).await?;
    let principal = principal_for_scope_user(&repo, Some(&user));
    ensure_repo_read(&state, &repo, &principal)?;
    let access = repo.access_for_principal(&principal);
    let request = state
        .metadata
        .request_by_id(&request_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if request.repo_id != repo.record.id || request.author_user_id != user.id {
        return Err(ApiError::not_found("request not found"));
    }
    if request.author_role != RequestActorRole::Public
        && !matches!(
            access.actor,
            RepositoryActor::Owner | RepositoryActor::Member
        )
    {
        return Err(ApiError::forbidden(
            "repo maintainer required to submit this request",
        ));
    }
    let head_oid = git_oid_request("head_oid", &input.head_oid)?;
    let stake_credits = input.stake_credits.unwrap_or(0);
    let mutation = state
        .metadata
        .submit_request(SubmitRequestInput {
            request_id,
            actor_user_id: user.id.clone(),
            expected_head_oid: head_oid,
            stake_credits,
            stake_ledger_entry_id: if stake_credits == 0 {
                None
            } else {
                Some(random_id("ledger_request_stake")?)
            },
            event_id: random_id("event_request_created")?,
            now_unix: unix_now()?,
        })
        .await?;
    let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
    let request = request_response(mutation.request, access, current_main_oid, Some(&user.id))?;
    publish_request_summary_refresh(&state, &repo, "request-submitted").await;
    Ok(Json(RequestMutationResponse { request }))
}

pub(crate) async fn comment_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<CommentRequestRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    if !request_permissions(&request, access, Some(&user.id)).can_comment {
        return Err(ApiError::forbidden("request author or maintainer required"));
    }
    let mutation = state
        .metadata
        .comment_request(CommentRequestInput {
            request_id,
            actor_user_id: user.id.clone(),
            event_id: random_id("event_request_comment")?,
            body: input.body,
            now_unix: unix_now()?,
        })
        .await?;
    let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
    let request = request_response(mutation.request, access, current_main_oid, Some(&user.id))?;
    Ok(Json(RequestMutationResponse { request }))
}

pub(crate) async fn mark_needs_response(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<NeedsResponseRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    ensure_maintainer(access)?;
    visible_request(&state, &repo, access, &request_id).await?;
    let mutation = state
        .metadata
        .mark_request_needs_response(MarkRequestNeedsResponseInput {
            request_id,
            actor_user_id: user.id.clone(),
            event_id: random_id("event_request_needs_response")?,
            body: input.body,
            now_unix: unix_now()?,
        })
        .await?;
    let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
    let request = request_response(mutation.request, access, current_main_oid, Some(&user.id))?;
    Ok(Json(RequestMutationResponse { request }))
}

pub(crate) async fn respond_to_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<RespondRequestRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    visible_request(&state, &repo, access, &request_id).await?;
    let mutation = state
        .metadata
        .respond_to_request(RespondToRequestInput {
            request_id,
            actor_user_id: user.id.clone(),
            event_id: random_id("event_request_response")?,
            body: input.body,
            now_unix: unix_now()?,
        })
        .await?;
    let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
    let request = request_response(mutation.request, access, current_main_oid, Some(&user.id))?;
    Ok(Json(RequestMutationResponse { request }))
}

pub(crate) async fn resolve_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<ResolveRequestRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    ensure_maintainer(access)?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    if !matches!(
        request.state,
        RequestState::Submitted | RequestState::NeedsResponse
    ) {
        return Err(ApiError::conflict(
            "request must be submitted before it can be resolved",
        ));
    }
    let now_unix = unix_now()?;
    let disposition = input.disposition.into();
    let settlement = settlement_for(request.stake_credits, disposition, now_unix);
    let mutation = state
        .metadata
        .resolve_request(ResolveRequestInput {
            request_id,
            actor_user_id: user.id.clone(),
            disposition,
            event_id: random_id("event_request_resolved")?,
            settlement_event_id: random_id("event_request_settled")?,
            refund_ledger_entry_id: ledger_id_for(settlement.refunded_credits, "refund")?,
            reward_ledger_entry_id: ledger_id_for(settlement.reward_credits, "reward")?,
            body: input.body,
            now_unix,
        })
        .await?;
    let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
    let request = request_response(mutation.request, access, current_main_oid, Some(&user.id))?;
    publish_request_summary_refresh(&state, &repo, "request-resolved").await;
    Ok(Json(RequestMutationResponse { request }))
}

pub(crate) async fn merge_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<MergeRequestRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    ensure_maintainer(access)?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    if request.state != RequestState::Submitted {
        return Err(ApiError::conflict(
            "request must be submitted before it can be merged",
        ));
    }
    let expected_main_oid = git_oid_request("expected_main_oid", &input.expected_main_oid)?;
    let expected_head_oid = git_oid_request("expected_head_oid", &input.expected_head_oid)?;
    let current_main_oid = current_main_oid_for_access(&state, &repo, access)?
        .ok_or_else(|| ApiError::conflict("repo has no main branch to merge into"))?;
    if current_main_oid != expected_main_oid {
        return Err(ApiError::conflict("main changed since merge was confirmed"));
    }
    if request.head_oid != expected_head_oid {
        return Err(ApiError::conflict(
            "request changed since merge was confirmed",
        ));
    }
    let mut update = clean_merge_update(
        &state,
        &owner,
        &repo_name,
        &repo,
        &request,
        &user.id,
        &current_main_oid,
    )
    .await?;
    update.base_git_snapshot_key = Some(
        repo.git_snapshot
            .as_ref()
            .map(|blob| blob.object_key.clone()),
    );
    update.base_config_hash = repo_config_fingerprint(&repo.repo_config)?;
    let uploaded_blobs = update
        .uploaded_blobs
        .iter()
        .cloned()
        .chain(std::iter::once(update.git_snapshot.clone()))
        .collect::<Vec<_>>();
    let now_unix = unix_now()?;
    let settlement = settlement_for(
        request.stake_credits,
        RequestDisposition::Accepted,
        now_unix,
    );
    let merge_input = MergeRequestInput {
        request_id,
        actor_user_id: user.id.clone(),
        expected_main_oid,
        current_main_oid: current_main_oid.clone(),
        expected_head_oid,
        event_id: random_id("event_request_merged")?,
        settlement_event_id: random_id("event_request_settled")?,
        refund_ledger_entry_id: ledger_id_for(settlement.refunded_credits, "refund")?,
        reward_ledger_entry_id: ledger_id_for(settlement.reward_credits, "reward")?,
        body: input.body,
        now_unix,
    };
    let maintainer_id = user.id.clone();
    let result = state
        .metadata
        .merge_request_with_repository_mutation(&owner, &repo_name, merge_input, move |repo| {
            apply_request_merge_update(repo, update, &maintainer_id).map_err(Into::into)
        })
        .await;
    let mutation = match result {
        Ok(mutation) => {
            let _repository_result = mutation.repository_result;
            mutation.request
        }
        Err(error) => {
            crate::state::best_effort_cleanup_rollback_source_blobs(&state, &uploaded_blobs).await;
            return Err(error.into());
        }
    };
    best_effort_drain_pending_source_blob_deletions(&state).await;
    let repo = find_repo(&state, &owner, &repo_name).await?;
    state
        .publish_repo_change(
            &repo.record.id,
            repo.record.change_version,
            "request-merged",
        )
        .await;
    let request = request_response(
        mutation.request,
        access,
        Some(current_main_oid),
        Some(&user.id),
    )?;
    Ok(Json(RequestMutationResponse { request }))
}

pub(crate) async fn repo_and_access(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
) -> Result<(StoredRepository, RepositoryAccess, Option<String>), ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    let user = optional_scope_user(state, headers).await?;
    let principal = user
        .as_ref()
        .map(|user| principal_for_scope_user(&repo, Some(user)))
        .unwrap_or_else(crate::domain::policy::Principal::public);
    ensure_repo_read(state, &repo, &principal)?;
    let access = repo.access_for_principal(&principal);
    Ok((repo, access, user.map(|user| user.id)))
}

pub(crate) async fn visible_request(
    state: &AppState,
    repo: &StoredRepository,
    access: RepositoryAccess,
    request_id: &str,
) -> Result<Request, ApiError> {
    let request = state
        .metadata
        .request_by_id(request_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if request.repo_id != repo.record.id || !request_visible_to_access(&request, access) {
        return Err(ApiError::not_found("request not found"));
    }
    Ok(request)
}

async fn publish_request_summary_refresh(
    state: &AppState,
    repo: &StoredRepository,
    reason: &'static str,
) {
    state
        .publish_repo_change(&repo.record.id, REQUEST_SUMMARY_REFRESH_VERSION, reason)
        .await;
}

fn request_response(
    request: Request,
    access: RepositoryAccess,
    current_main_oid: Option<String>,
    viewer_user_id: Option<&str>,
) -> Result<RequestSummaryResponse, ApiError> {
    let decision = request_permissions(&request, access, viewer_user_id);
    let permissions = RequestPermissionsResponse {
        can_comment: decision.can_comment,
        can_pull_branch: decision.can_pull_branch,
        can_push_branch: decision.can_push_branch,
        can_delete: decision.can_delete,
        can_mark_needs_response: decision.can_mark_needs_response,
        can_respond: decision.can_respond,
        can_resolve: decision.can_resolve,
        can_merge: decision.can_merge,
    };
    let decision = request_mergeability(&request, access);
    let mergeability = RequestMergeabilityResponse {
        status: decision.status,
        current_main_oid: current_main_oid.map(git_oid_response).transpose()?,
        request_head_oid: git_oid_response(request.head_oid.clone())?,
        reason: decision.reason.map(str::to_string),
    };
    request_summary_response(request, permissions, mergeability)
}

fn current_main_oid_for_access(
    state: &AppState,
    repo: &StoredRepository,
    access: RepositoryAccess,
) -> Result<Option<String>, ApiError> {
    match access.actor {
        RepositoryActor::Owner | RepositoryActor::Member => {
            if let Some(snapshot) = repo.git_snapshot.as_ref() {
                let repo_path = cached_raw_git_snapshot_repo(state, snapshot)?;
                return main_oid_from_git_repo(&repo_path);
            }
            projection_main_oid(state, repo, ProjectionViewKey::Private)
        }
        RepositoryActor::Public => projection_main_oid(state, repo, ProjectionViewKey::Public),
    }
}

fn current_main_oid_for_audience(
    state: &AppState,
    repo: &StoredRepository,
    audience: RequestAudience,
) -> Result<Option<String>, ApiError> {
    match audience {
        RequestAudience::Public => projection_main_oid(state, repo, ProjectionViewKey::Public),
        RequestAudience::Private => {
            if let Some(snapshot) = repo.git_snapshot.as_ref() {
                let repo_path = cached_raw_git_snapshot_repo(state, snapshot)?;
                return main_oid_from_git_repo(&repo_path);
            }
            projection_main_oid(state, repo, ProjectionViewKey::Private)
        }
    }
}

fn projection_main_oid(
    state: &AppState,
    repo: &StoredRepository,
    view_key: ProjectionViewKey,
) -> Result<Option<String>, ApiError> {
    let projection = project_graph(&repo.policy, &repo.graph, &repo.visibility_events, view_key);
    if projection.commits.is_empty() {
        return Ok(None);
    }
    let repo_path = projection_bare_repo_for_state(state, &projection)?;
    main_oid_from_git_repo(&repo_path)
}

fn main_oid_from_git_repo(repo_path: &std::path::Path) -> Result<Option<String>, ApiError> {
    let main_ref = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
    Ok(git_refs(repo_path)?
        .into_iter()
        .find_map(|(refname, oid)| (refname == main_ref).then_some(oid)))
}

fn ensure_maintainer(access: RepositoryAccess) -> Result<(), ApiError> {
    if matches!(
        access.actor,
        RepositoryActor::Owner | RepositoryActor::Member
    ) {
        Ok(())
    } else {
        Err(ApiError::forbidden("repo maintainer required"))
    }
}

fn ledger_id_for(amount: u32, kind: &str) -> Result<Option<String>, ApiError> {
    if amount == 0 {
        Ok(None)
    } else {
        random_id(&format!("ledger_request_{kind}")).map(Some)
    }
}

fn random_id(prefix: &str) -> Result<String, ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to create {prefix} id: {error}"))
    })?;
    Ok(format!("{prefix}_{}", hex::encode(bytes)))
}
