use crate::{
    auth::scope::{optional_scope_user, principal_for_scope_user, require_scope_user},
    config::DEFAULT_GIT_BRANCH,
    domain::{
        projection::{ProjectionViewKey, project_graph},
        requests::{
            AssessRequestInput, CloseRequestInput, CloseRequestMutation, MarkRequestReadyInput,
            MergeRequestInput, REQUEST_LIST_DEFAULT_PAGE_SIZE, REQUEST_LIST_MAX_PAGE_SIZE, Request,
            RequestActorRole, RequestAssessmentOutcome, RequestAudience, RequestReviewExitReason,
            ReturnRequestToWorkingInput, SetRequestHoldInput, StartRequestInput,
            UpdateRequestDescriptionInput, canonical_request_ref, request_actor_role,
            request_mergeability, request_permissions, request_visible_audiences,
            request_visible_to_access,
        },
        store::{RepositoryAccess, RepositoryActor, StoredRepository},
    },
    error::ApiError,
    git::{
        import::git_refs, request_merge::prepare_request_merge,
        request_refs::delete_request_ref_from_store, storage::cached_raw_git_repo,
        upload::projection_bare_repo_for_state,
    },
    http::responses::*,
    persistence::unix_now,
    state::{AppState, ensure_repo_read, find_repo},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct RequestListQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

pub(crate) async fn list_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<RequestListQuery>,
) -> Result<Json<RequestListResponse>, ApiError> {
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let after_id = query
        .cursor
        .as_deref()
        .map(parse_request_list_cursor)
        .transpose()?;
    let limit = query
        .limit
        .unwrap_or(REQUEST_LIST_DEFAULT_PAGE_SIZE)
        .clamp(1, REQUEST_LIST_MAX_PAGE_SIZE);
    let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
    let mut requests = state
        .metadata
        .request_list_page(
            &repo.record.id,
            request_visible_audiences(access),
            after_id.as_deref(),
            (limit + 1) as u64,
        )
        .await?;
    let has_more = requests.len() > limit;
    requests.truncate(limit);
    let next_cursor = if has_more {
        requests
            .last()
            .map(|request| encode_request_list_cursor(&request.id))
    } else {
        None
    };
    let requests = requests
        .into_iter()
        .map(|request| request_list_item_response(request, access, current_main_oid.clone()))
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(Json(RequestListResponse {
        requests,
        next_cursor,
    }))
}

fn parse_request_list_cursor(value: &str) -> Result<String, ApiError> {
    value
        .strip_prefix("v1:")
        .filter(|id| !id.is_empty() && !id.contains(':'))
        .map(str::to_string)
        .ok_or_else(|| ApiError::bad_request("invalid request list cursor"))
}

fn encode_request_list_cursor(last_id: &str) -> String {
    format!("v1:{last_id}")
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
    let request = request_response(request, access, current_main_oid, viewer_user_id.as_deref())?;

    Ok(Json(RequestDetailResponse { request }))
}

pub(crate) async fn mark_request_ready(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<ReadyRequestRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    let uses_credits = request.author_role == RequestActorRole::Public;
    let mutation = state
        .metadata
        .mark_request_ready(MarkRequestReadyInput {
            request_id: request.id,
            actor_user_id: user.id.clone(),
            actor_is_author: false,
            actor_can_mutate: false,
            stake_credits: input.stake_credits,
            public_ready_count: 0,
            ready_queue_version: 0,
            event_id: random_id("event_request_ready")?,
            stake_ledger_entry_id: uses_credits
                .then(|| random_id("ledger_review_stake"))
                .transpose()?,
            now_unix: unix_now()?,
        })
        .await?;
    lifecycle_response(
        &state,
        &repo,
        access,
        &user.id,
        mutation.request,
        "request-ready",
    )
    .await
}

pub(crate) async fn return_request_to_working(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    let mutation = state
        .metadata
        .return_request_to_working(ReturnRequestToWorkingInput {
            request_id: request.id,
            actor_user_id: user.id.clone(),
            actor_is_author: false,
            actor_is_maintainer: false,
            actor_can_mutate: false,
            reason: RequestReviewExitReason::AuthorReturned,
            event_id: random_id("event_request_working")?,
            now_unix: unix_now()?,
        })
        .await?;
    lifecycle_response(
        &state,
        &repo,
        access,
        &user.id,
        mutation.request,
        "request-working",
    )
    .await
}

pub(crate) async fn hold_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    set_hold(state, headers, owner, repo_name, request_id, true).await
}

pub(crate) async fn release_request_hold(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    set_hold(state, headers, owner, repo_name, request_id, false).await
}

async fn set_hold(
    state: AppState,
    headers: HeaderMap,
    owner: String,
    repo_name: String,
    request_id: String,
    held: bool,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    let mutation = state
        .metadata
        .set_request_hold(SetRequestHoldInput {
            request_id: request.id,
            actor_user_id: user.id.clone(),
            actor_is_maintainer: false,
            held,
            event_id: random_id(if held {
                "event_request_held"
            } else {
                "event_request_hold_released"
            })?,
            now_unix: unix_now()?,
        })
        .await?;
    lifecycle_response(
        &state,
        &repo,
        access,
        &user.id,
        mutation.request,
        if held {
            "request-held"
        } else {
            "request-unheld"
        },
    )
    .await
}

pub(crate) async fn request_changes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    let mutation = state
        .metadata
        .return_request_to_working(ReturnRequestToWorkingInput {
            request_id: request.id,
            actor_user_id: user.id.clone(),
            actor_is_author: false,
            actor_is_maintainer: false,
            actor_can_mutate: false,
            reason: RequestReviewExitReason::ChangesRequested,
            event_id: random_id("event_request_changes_requested")?,
            now_unix: unix_now()?,
        })
        .await?;
    lifecycle_response(
        &state,
        &repo,
        access,
        &user.id,
        mutation.request,
        "request-changes-requested",
    )
    .await
}

pub(crate) async fn assess_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<AssessRequestRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    let ids = settlement_ids(&request, input.outcome)?;
    let mutation = state
        .metadata
        .assess_request(AssessRequestInput {
            request_id: request.id,
            actor_user_id: user.id.clone(),
            actor_is_maintainer: false,
            outcome: input.outcome,
            body_markdown: input.body_markdown,
            assessed_event_id: random_id("event_request_assessed")?,
            settled_event_id: ids.settled_event_id,
            refund_ledger_entry_id: ids.refund_ledger_entry_id,
            reward_ledger_entry_id: ids.reward_ledger_entry_id,
            now_unix: unix_now()?,
        })
        .await?;
    lifecycle_response(
        &state,
        &repo,
        access,
        &user.id,
        mutation.request,
        "request-assessed",
    )
    .await
}

pub(crate) async fn merge_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    if !request_permissions(&request, access, Some(&user.id)).can_merge {
        if matches!(access.actor, RepositoryActor::Public) {
            return Err(ApiError::forbidden("repo maintainer required"));
        }
        return Err(ApiError::conflict("request cannot be merged"));
    }
    let ids = settlement_ids(&request, RequestAssessmentOutcome::Accepted)?;
    let prepared =
        prepare_request_merge(&state, &owner, &repo_name, &user.id, &repo, &request).await?;
    let durable_objects = prepared.durable_objects().to_vec();
    let mutation = match state
        .metadata
        .merge_request_content(
            &owner,
            &repo_name,
            &prepared.expected_manifest_key,
            prepared.expected_repo_change_version,
            &prepared.prepared_request_head_oid,
            prepared.update.into_reviewed_update(),
            MergeRequestInput {
                request_id: request.id,
                actor_user_id: user.id.clone(),
                actor_is_maintainer: false,
                merged_head_oid: String::new(),
                merged_main_oid: String::new(),
                merged_event_id: random_id("event_request_merged")?,
                assessed_event_id: random_id("event_request_merge_assessed")?,
                settled_event_id: ids.settled_event_id,
                refund_ledger_entry_id: ids.refund_ledger_entry_id,
                reward_ledger_entry_id: ids.reward_ledger_entry_id,
                now_unix: unix_now()?,
            },
        )
        .await
    {
        Ok(mutation) => mutation,
        Err(error) => {
            crate::state::best_effort_cleanup_rollback_source_blobs(&state, &durable_objects).await;
            return Err(error.into());
        }
    };
    state
        .publish_repo_change(
            &repo.record.id,
            mutation.git_head.change_version,
            "request-merged",
        )
        .await;
    let committed_repo = find_repo(&state, &owner, &repo_name).await?;
    lifecycle_response(
        &state,
        &committed_repo,
        access,
        &user.id,
        mutation.request.request,
        "request-merged",
    )
    .await
}

#[derive(Default)]
struct SettlementIds {
    settled_event_id: Option<String>,
    refund_ledger_entry_id: Option<String>,
    reward_ledger_entry_id: Option<String>,
}

fn settlement_ids(
    request: &Request,
    outcome: RequestAssessmentOutcome,
) -> Result<SettlementIds, ApiError> {
    if request.current_stake_credits == 0 {
        return Ok(SettlementIds::default());
    }
    Ok(SettlementIds {
        settled_event_id: Some(random_id("event_request_settled")?),
        refund_ledger_entry_id: matches!(
            outcome,
            RequestAssessmentOutcome::Accepted | RequestAssessmentOutcome::Neutral
        )
        .then(|| random_id("ledger_review_stake_refund"))
        .transpose()?,
        reward_ledger_entry_id: (outcome == RequestAssessmentOutcome::Accepted)
            .then(|| random_id("ledger_assessment_reward"))
            .transpose()?,
    })
}

async fn lifecycle_response(
    state: &AppState,
    repo: &StoredRepository,
    access: RepositoryAccess,
    viewer_user_id: &str,
    request: Request,
    refresh_reason: &'static str,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let current_main_oid = current_main_oid_for_access(state, repo, access)?;
    let request = request_response(request, access, current_main_oid, Some(viewer_user_id))?;
    state
        .publish_request_summary_refresh(&repo.record.id, refresh_reason)
        .await;
    Ok(Json(RequestMutationResponse { request }))
}

pub(crate) async fn close_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestCloseResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    if !request_permissions(&request, access, Some(&user.id)).can_close {
        return Err(ApiError::forbidden("request close access required"));
    }
    let request_ref = canonical_request_ref(&request.name);
    let mutation = state
        .metadata
        .close_request(CloseRequestInput {
            request_id: request.id,
            actor_user_id: user.id.clone(),
            actor_can_close: false,
            event_id: random_id("event_request_closed")?,
            now_unix: unix_now()?,
        })
        .await?;
    match mutation {
        CloseRequestMutation::DeletedDraft { .. } => {
            delete_request_ref_from_store(&state, &owner, &repo_name, &request_ref)?;
            state
                .publish_request_summary_refresh(&repo.record.id, "request-deleted")
                .await;
            Ok(Json(RequestCloseResponse {
                deleted: true,
                request: None,
            }))
        }
        CloseRequestMutation::Completed { request, .. } => {
            let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
            let request = request_response(request, access, current_main_oid, Some(&user.id))?;
            state
                .publish_request_summary_refresh(&repo.record.id, "request-closed")
                .await;
            Ok(Json(RequestCloseResponse {
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
            event_id: random_id("event_request_started")?,
            now_unix,
        })
        .await?;
    let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
    let request = request_response(mutation.request, access, current_main_oid, Some(&user.id))?;
    state
        .publish_request_summary_refresh(&repo.record.id, "request-started")
        .await;
    Ok(Json(RequestMutationResponse { request }))
}

pub(crate) async fn update_request_description(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<UpdateRequestDescriptionRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, &request_id).await?;
    let mutation = state
        .metadata
        .update_request_description_with_review_invalidation(UpdateRequestDescriptionInput {
            request_id: request.id,
            actor_user_id: user.id.clone(),
            actor_can_edit_description: false,
            event_id: random_id("event_request_description_edited")?,
            description_markdown: input.description_markdown,
            now_unix: unix_now()?,
        })
        .await?;
    let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
    let request = request_response(mutation.request, access, current_main_oid, Some(&user.id))?;
    state
        .publish_request_summary_refresh(&repo.record.id, "request-description-edited")
        .await;
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

fn request_response(
    request: Request,
    access: RepositoryAccess,
    current_main_oid: Option<String>,
    viewer_user_id: Option<&str>,
) -> Result<RequestSummaryResponse, ApiError> {
    let decision = request_permissions(&request, access, viewer_user_id);
    let permissions = RequestPermissionsResponse {
        can_open_discussion: decision.can_open_discussion,
        can_reply_to_discussion: decision.can_reply_to_discussion,
        can_edit_description: decision.can_edit_description,
        can_pull_branch: decision.can_pull_branch,
        can_push_branch: decision.can_push_branch,
        can_mark_ready: decision.can_mark_ready,
        can_return_to_working: decision.can_return_to_working,
        can_manage_invitees: decision.can_manage_invitees,
        can_hold: decision.can_hold,
        can_assess: decision.can_assess,
        can_close: decision.can_close,
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
            if let Some(head) = repo.git_head.as_ref() {
                let repo_path = cached_raw_git_repo(state, &head.manifest)?;
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
            if let Some(head) = repo.git_head.as_ref() {
                let repo_path = cached_raw_git_repo(state, &head.manifest)?;
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

pub(crate) fn random_id(prefix: &str) -> Result<String, ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to create {prefix} id: {error}"))
    })?;
    Ok(format!("{prefix}_{}", hex::encode(bytes)))
}
