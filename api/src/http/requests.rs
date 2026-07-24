use crate::{
    auth::scope::{optional_scope_user, principal_for_scope_user, require_scope_user},
    config::DEFAULT_GIT_BRANCH,
    domain::{
        projection::{ProjectionViewKey, project_graph},
        requests::{
            CloseRequestInput, CloseRequestMutation, REQUEST_LIST_DEFAULT_PAGE_SIZE,
            REQUEST_LIST_MAX_PAGE_SIZE, Request, RequestAudience, StartRequestInput,
            UpdateRequestDescriptionInput, canonical_request_ref, request_actor_role,
            request_mergeability, request_permissions, request_visible_audiences,
            request_visible_to_access,
        },
        store::{RepositoryAccess, RepositoryActor, StoredRepository},
    },
    error::ApiError,
    git::{
        import::git_refs, request_refs::delete_request_ref_from_store,
        storage::cached_raw_git_repo, upload::projection_bare_repo_for_state,
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
        .update_request_description(UpdateRequestDescriptionInput {
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
