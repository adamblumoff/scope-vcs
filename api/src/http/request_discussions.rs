use crate::{
    auth::scope::require_scope_user,
    error::ApiError,
    http::{
        request_review::{RequestRevisionCommitVisibility, validate_request_discussion_anchor},
        requests::*,
        responses::*,
    },
    persistence::unix_now,
    product_analytics::ProductEvent,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use scope_api_contract::{
    CreateRequestDiscussionReplyRequest, CreateRequestDiscussionRequest, GitOid,
    MarkRequestDiscussionReadRequest, ReopenAndReplyRequest, RequestActivityPageResponse,
    RequestDiscussionAnchor, RequestDiscussionChangesResponse, RequestDiscussionMutationResponse,
    RequestDiscussionPageResponse, RequestDiscussionReadResponse,
    RequestDiscussionRepliesPageResponse, RequestDiscussionReplyMutationResponse,
    RequestDiscussionReplyResponse, RequestDiscussionSummaryResponse,
};
use scope_domain::requests::{
    CreateRequestDiscussionInput, CreateRequestDiscussionReplyInput,
    MarkRequestDiscussionReadInput, REQUEST_ACTIVITY_PAGE_MAX_EVENTS,
    ReopenAndReplyToRequestDiscussionInput, RequestViewer, request_actor_role, request_policy,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

const DEFAULT_DISCUSSION_LIMIT: usize = 25;
const MAX_DISCUSSION_LIMIT: usize = 100;
const DEFAULT_REPLY_LIMIT: u64 = 50;
const MAX_REPLY_LIMIT: u64 = 100;

#[derive(Debug, Deserialize)]
pub(crate) struct DiscussionListQuery {
    cursor: Option<String>,
    discussion: Option<String>,
    limit: Option<usize>,
    revision: Option<String>,
    commit: Option<String>,
    include_revision_anchor: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DiscussionChangesQuery {
    after: Option<u64>,
    limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DiscussionRepliesQuery {
    before: Option<u64>,
    limit: Option<u64>,
    parent_reply_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ActivityQuery {
    after: Option<u64>,
    latest: Option<bool>,
    limit: Option<usize>,
}

pub(crate) async fn list_discussions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Query(query): Query<DiscussionListQuery>,
) -> Result<Json<RequestDiscussionPageResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(
        &state,
        &repo,
        access,
        viewer_user_id.as_deref(),
        &request_id,
    )
    .await?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_DISCUSSION_LIMIT)
        .clamp(1, MAX_DISCUSSION_LIMIT);
    let cursor = query
        .cursor
        .as_deref()
        .map(parse_discussion_cursor)
        .transpose()?;
    if query.commit.is_some() && query.revision.is_none() {
        return Err(ApiError::bad_request(
            "filtering discussions by commit requires a revision",
        ));
    }
    let commit_oid = query
        .commit
        .map(GitOid::try_from)
        .transpose()
        .map_err(ApiError::bad_request)?
        .map(String::from);
    let snapshot_version = cursor
        .as_ref()
        .map(|cursor| cursor.snapshot_version)
        .unwrap_or(request.activity_version);
    let batch = state
        .metadata
        .requests()
        .request_discussions_page(scope_postgres::db::RequestDiscussionsPageQuery {
            request_id: &request.id,
            viewer_user_id: viewer_user_id.as_deref(),
            snapshot_version,
            cursor: cursor
                .as_ref()
                .map(|cursor| (cursor.position, cursor.id.clone())),
            discussion_id: query.discussion.as_deref(),
            anchor_revision_id: query.revision.as_deref(),
            anchor_commit_oid: commit_oid.as_deref(),
            include_revision_anchor: query.include_revision_anchor.unwrap_or(false),
            limit: (limit + 1) as u64,
        })
        .await?;
    let mut discussions = batch.discussions;
    let has_more = discussions.len() > limit;
    discussions.truncate(limit);
    let next_cursor = has_more
        .then(|| {
            discussions.last().map(|model| {
                encode_discussion_cursor(
                    snapshot_version,
                    model.discussion.opened_position,
                    &model.discussion.id,
                )
            })
        })
        .flatten();
    let projection = DiscussionProjection {
        state: &state,
        owner: &owner,
        repo_name: &repo_name,
        request: &request,
        repo: &repo,
        access,
    };
    let discussions = discussion_summaries(&projection, discussions, &batch.users).await?;
    Ok(Json(RequestDiscussionPageResponse {
        discussions,
        next_cursor,
        snapshot_version,
    }))
}

pub(crate) async fn create_discussion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<CreateRequestDiscussionRequest>,
) -> Result<Json<RequestDiscussionMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let actor_user_id = user.id.clone();
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, Some(&user.id), &request_id).await?;
    let anchor = match input.anchor {
        Some(anchor) => Some(
            validate_request_discussion_anchor(
                &state, &owner, &repo_name, &repo, access, &request, anchor,
            )
            .await?,
        ),
        None => None,
    };
    let mutation = state
        .metadata
        .requests()
        .create_request_discussion(CreateRequestDiscussionInput {
            request_id: request.id.clone(),
            id: random_id("discussion")?,
            actor_user_id: actor_user_id.clone(),
            actor_can_participate: false,
            client_discussion_id: input.client_discussion_id,
            body_markdown: input.body_markdown,
            anchor,
            now_unix: unix_now()?,
        })
        .await?;
    if mutation.created {
        state
            .product_analytics
            .capture(ProductEvent::discussion_created(
                &actor_user_id,
                request.audience,
                request_actor_role(access),
                mutation.discussion.anchor.is_some(),
            ));
    }
    let through_position = mutation.discussion.last_activity_position;
    let discussion_id = mutation.discussion.id.clone();
    let projection = DiscussionProjection {
        state: &state,
        owner: &owner,
        repo_name: &repo_name,
        request: &request,
        repo: &repo,
        access,
    };
    let discussion = load_one_summary(&projection, &discussion_id, Some(&actor_user_id)).await?;
    state
        .publish_request_timeline_change(
            &repo.record.id,
            request.id,
            discussion_id,
            through_position,
            request.audience,
        )
        .await;
    Ok(Json(RequestDiscussionMutationResponse { discussion }))
}

pub(crate) async fn list_replies(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id, discussion_id)): Path<(String, String, String, String)>,
    Query(query): Query<DiscussionRepliesQuery>,
) -> Result<Json<RequestDiscussionRepliesPageResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_and_access(&state, &headers, &owner, &repo_name).await?;
    visible_request(
        &state,
        &repo,
        access,
        viewer_user_id.as_deref(),
        &request_id,
    )
    .await?;
    ensure_discussion_in_request(&state, &request_id, &discussion_id).await?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_REPLY_LIMIT)
        .clamp(1, MAX_REPLY_LIMIT);
    let (mut replies, users) = state
        .metadata
        .requests()
        .request_discussion_replies(
            &discussion_id,
            query.parent_reply_id.as_deref(),
            query.before,
            limit + 1,
        )
        .await?;
    let has_more = replies.len() as u64 > limit;
    if has_more {
        replies.remove(0);
    }
    let next_before_position = has_more
        .then(|| replies.first().map(|model| model.reply.position))
        .flatten();
    let replies = replies
        .into_iter()
        .map(|reply| reply_read_response(reply, &users))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(RequestDiscussionRepliesPageResponse {
        replies,
        next_before_position,
    }))
}

pub(crate) async fn create_reply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id, discussion_id)): Path<(String, String, String, String)>,
    Json(input): Json<CreateRequestDiscussionReplyRequest>,
) -> Result<Json<RequestDiscussionReplyMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let actor_user_id = user.id.clone();
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, Some(&user.id), &request_id).await?;
    let mutation = state
        .metadata
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyInput {
            request_id: request.id.clone(),
            discussion_id: discussion_id.clone(),
            id: random_id("discussion_reply")?,
            actor_user_id: actor_user_id.clone(),
            actor_can_participate: false,
            client_reply_id: input.client_reply_id,
            body_markdown: input.body_markdown,
            reply_to_reply_id: input.reply_to_reply_id,
            now_unix: unix_now()?,
        })
        .await?;
    let projection = DiscussionProjection {
        state: &state,
        owner: &owner,
        repo_name: &repo_name,
        request: &request,
        repo: &repo,
        access,
    };
    reply_mutation_response(
        &projection,
        mutation.discussion.id,
        mutation.reply,
        &actor_user_id,
    )
    .await
}

pub(crate) async fn resolve_discussion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id, discussion_id)): Path<(String, String, String, String)>,
) -> Result<Json<RequestDiscussionMutationResponse>, ApiError> {
    transition_discussion(
        state,
        headers,
        owner,
        repo_name,
        request_id,
        discussion_id,
        true,
    )
    .await
}

pub(crate) async fn reopen_discussion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id, discussion_id)): Path<(String, String, String, String)>,
) -> Result<Json<RequestDiscussionMutationResponse>, ApiError> {
    transition_discussion(
        state,
        headers,
        owner,
        repo_name,
        request_id,
        discussion_id,
        false,
    )
    .await
}

pub(crate) async fn reopen_and_reply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id, discussion_id)): Path<(String, String, String, String)>,
    Json(input): Json<ReopenAndReplyRequest>,
) -> Result<Json<RequestDiscussionReplyMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let actor_user_id = user.id.clone();
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, Some(&user.id), &request_id).await?;
    let mutation = state
        .metadata
        .requests()
        .reopen_and_reply_to_request_discussion(ReopenAndReplyToRequestDiscussionInput {
            request_id: request.id.clone(),
            discussion_id: discussion_id.clone(),
            reply_id: random_id("discussion_reply")?,
            actor_user_id: actor_user_id.clone(),
            actor_is_maintainer: false,
            actor_can_transition: false,
            actor_can_participate: false,
            event_id: random_id("event_request_discussion_reopened")?,
            client_reply_id: input.client_reply_id,
            body_markdown: input.body_markdown,
            reply_to_reply_id: input.reply_to_reply_id,
            now_unix: unix_now()?,
        })
        .await?;
    let projection = DiscussionProjection {
        state: &state,
        owner: &owner,
        repo_name: &repo_name,
        request: &request,
        repo: &repo,
        access,
    };
    reply_mutation_response(
        &projection,
        mutation.discussion.id,
        mutation.reply,
        &actor_user_id,
    )
    .await
}

pub(crate) async fn mark_read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id, discussion_id)): Path<(String, String, String, String)>,
    Json(input): Json<MarkRequestDiscussionReadRequest>,
) -> Result<Json<RequestDiscussionReadResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    visible_request(&state, &repo, access, Some(&user.id), &request_id).await?;
    ensure_discussion_in_request(&state, &request_id, &discussion_id).await?;
    let state = state
        .metadata
        .requests()
        .mark_request_discussion_read(MarkRequestDiscussionReadInput {
            discussion_id,
            user_id: user.id,
            through_position: input.through_position,
            now_unix: unix_now()?,
        })
        .await?;
    Ok(Json(RequestDiscussionReadResponse {
        read_through_position: state.read_through_position,
    }))
}

pub(crate) async fn changed_discussions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Query(query): Query<DiscussionChangesQuery>,
) -> Result<Json<RequestDiscussionChangesResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(
        &state,
        &repo,
        access,
        viewer_user_id.as_deref(),
        &request_id,
    )
    .await?;
    let limit = query.limit.unwrap_or(100).clamp(1, 100);
    let mut batch = state
        .metadata
        .requests()
        .changed_request_discussions(
            &request.id,
            viewer_user_id.as_deref(),
            query.after.unwrap_or(0),
            limit + 1,
        )
        .await?;
    let has_more = batch.discussions.len() > limit as usize;
    batch.discussions.truncate(limit as usize);
    let through_position = batch
        .discussions
        .last()
        .map(|model| model.discussion.last_activity_position)
        .unwrap_or(request.activity_version);
    let projection = DiscussionProjection {
        state: &state,
        owner: &owner,
        repo_name: &repo_name,
        request: &request,
        repo: &repo,
        access,
    };
    let discussions = discussion_summaries(&projection, batch.discussions, &batch.users).await?;
    Ok(Json(RequestDiscussionChangesResponse {
        discussions,
        through_position,
        has_more,
    }))
}

pub(crate) async fn activity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<RequestActivityPageResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(
        &state,
        &repo,
        access,
        viewer_user_id.as_deref(),
        &request_id,
    )
    .await?;
    let is_invitee = match viewer_user_id.as_deref() {
        Some(user_id) => {
            state
                .metadata
                .requests()
                .request_is_invitee(&request.id, user_id)
                .await?
        }
        None => false,
    };
    if !request_policy(
        &request,
        RequestViewer::new(access, viewer_user_id.as_deref(), is_invitee),
    )
    .activity_stream_visible
    {
        return Err(ApiError::not_found("request not found"));
    }
    let latest = query.latest.unwrap_or(false);
    let limit = query
        .limit
        .unwrap_or(REQUEST_ACTIVITY_PAGE_MAX_EVENTS)
        .clamp(1, REQUEST_ACTIVITY_PAGE_MAX_EVENTS);
    let events = if latest {
        state
            .metadata
            .requests()
            .latest_request_events(&request.id, limit as u64)
            .await?
    } else {
        state
            .metadata
            .requests()
            .request_events_after_position(&request.id, query.after.unwrap_or(0), limit as u64)
            .await?
    };
    let users = state
        .metadata
        .requests()
        .users_by_ids(events.iter().map(|event| event.actor_user_id.clone()))
        .await?;
    let through_position = if latest {
        request.activity_version
    } else {
        events
            .last()
            .map(|event| event.position)
            .unwrap_or(request.activity_version)
    };
    let events = events
        .into_iter()
        .map(|event| {
            let actor = request_actor_summary_response(&event.actor_user_id, &users)?;
            Ok(request_event_response(event, actor))
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(RequestActivityPageResponse {
        events,
        through_position,
    }))
}

async fn transition_discussion(
    state: AppState,
    headers: HeaderMap,
    owner: String,
    repo_name: String,
    request_id: String,
    discussion_id: String,
    resolve: bool,
) -> Result<Json<RequestDiscussionMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let actor_user_id = user.id.clone();
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, Some(&user.id), &request_id).await?;
    let discussion = if resolve {
        state
            .metadata
            .requests()
            .resolve_request_discussion(
                request.id.clone(),
                discussion_id.clone(),
                actor_user_id.clone(),
                random_id("event_request_discussion_resolved")?,
                unix_now()?,
            )
            .await?
    } else {
        state
            .metadata
            .requests()
            .reopen_request_discussion(
                request.id.clone(),
                discussion_id.clone(),
                actor_user_id.clone(),
                random_id("event_request_discussion_reopened")?,
                unix_now()?,
            )
            .await?
    };
    if resolve {
        state
            .product_analytics
            .capture(ProductEvent::discussion_resolved(
                &actor_user_id,
                request.audience,
                request_actor_role(access),
            ));
    }
    let through_position = discussion.last_activity_position;
    let projection = DiscussionProjection {
        state: &state,
        owner: &owner,
        repo_name: &repo_name,
        request: &request,
        repo: &repo,
        access,
    };
    let discussion = load_one_summary(&projection, &discussion_id, Some(&actor_user_id)).await?;
    state
        .publish_request_timeline_change(
            &repo.record.id,
            request.id,
            discussion_id,
            through_position,
            request.audience,
        )
        .await;
    Ok(Json(RequestDiscussionMutationResponse { discussion }))
}

async fn reply_mutation_response(
    projection: &DiscussionProjection<'_>,
    discussion_id: String,
    reply: scope_domain::requests::RequestDiscussionReply,
    actor_user_id: &str,
) -> Result<Json<RequestDiscussionReplyMutationResponse>, ApiError> {
    let discussion = load_one_summary(projection, &discussion_id, Some(actor_user_id)).await?;
    let users = projection
        .state
        .metadata
        .requests()
        .users_by_ids([reply.author_user_id.clone()])
        .await?;
    let child_reply_count = projection
        .state
        .metadata
        .requests()
        .request_discussion_reply_child_count(&reply.id)
        .await?;
    let response = reply_response(reply.clone(), child_reply_count, &users)?;
    projection
        .state
        .publish_request_timeline_change(
            &projection.repo.record.id,
            projection.request.id.clone(),
            discussion_id,
            reply.position,
            projection.request.audience,
        )
        .await;
    Ok(Json(RequestDiscussionReplyMutationResponse {
        discussion,
        reply: response,
    }))
}

async fn load_one_summary(
    projection: &DiscussionProjection<'_>,
    discussion_id: &str,
    viewer_user_id: Option<&str>,
) -> Result<RequestDiscussionSummaryResponse, ApiError> {
    let (model, users) = projection
        .state
        .metadata
        .requests()
        .request_discussion(&projection.request.id, discussion_id, viewer_user_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request discussion not found"))?;
    let visibility = discussion_anchor_visibility(
        projection,
        std::iter::once(model.discussion.anchor.as_ref()),
    )
    .await;
    let anchor = discussion_anchor_response(model.discussion.anchor.clone(), &visibility);
    discussion_summary(model, &users, anchor)
}

async fn ensure_discussion_in_request(
    state: &AppState,
    request_id: &str,
    discussion_id: &str,
) -> Result<(), ApiError> {
    state
        .metadata
        .requests()
        .request_discussion(request_id, discussion_id, None)
        .await?
        .ok_or_else(|| ApiError::not_found("request discussion not found"))?;
    Ok(())
}

fn discussion_summary(
    model: scope_postgres::db::RequestDiscussionReadModel,
    users: &BTreeMap<String, scope_domain::store::UserAccount>,
    anchor: Option<RequestDiscussionAnchor>,
) -> Result<RequestDiscussionSummaryResponse, ApiError> {
    Ok(RequestDiscussionSummaryResponse {
        id: model.discussion.id,
        request_id: model.discussion.request_id,
        client_discussion_id: model.discussion.client_discussion_id,
        opened_position: model.discussion.opened_position,
        last_activity_position: model.discussion.last_activity_position,
        author: request_actor_summary_response(&model.discussion.author_user_id, users)?,
        body_markdown: model.discussion.body_markdown,
        anchor,
        status: model.discussion.status.into(),
        reply_count: model.reply_count,
        unread_count: model.unread_count,
        latest_replies: model
            .latest_replies
            .into_iter()
            .map(|reply| reply_read_response(reply, users))
            .collect::<Result<Vec<_>, _>>()?,
        created_at_unix: model.discussion.created_at_unix,
        resolved_at_unix: model.discussion.resolved_at_unix,
        resolved_by: model
            .discussion
            .resolved_by_user_id
            .as_deref()
            .map(|id| request_actor_summary_response(id, users))
            .transpose()?,
    })
}

struct DiscussionProjection<'a> {
    state: &'a AppState,
    owner: &'a str,
    repo_name: &'a str,
    request: &'a scope_domain::requests::Request,
    repo: &'a scope_domain::store::StoredRepository,
    access: scope_domain::store::RepositoryAccess,
}

async fn discussion_summaries(
    projection: &DiscussionProjection<'_>,
    models: Vec<scope_postgres::db::RequestDiscussionReadModel>,
    users: &BTreeMap<String, scope_domain::store::UserAccount>,
) -> Result<Vec<RequestDiscussionSummaryResponse>, ApiError> {
    let visibility = discussion_anchor_visibility(
        projection,
        models.iter().map(|model| model.discussion.anchor.as_ref()),
    )
    .await;
    let mut summaries = Vec::with_capacity(models.len());
    for model in models {
        let anchor = discussion_anchor_response(model.discussion.anchor.clone(), &visibility);
        summaries.push(discussion_summary(model, users, anchor)?);
    }
    Ok(summaries)
}

async fn discussion_anchor_visibility<'a>(
    projection: &DiscussionProjection<'_>,
    anchors: impl Iterator<Item = Option<&'a scope_domain::requests::RequestDiscussionAnchor>>,
) -> BTreeSet<(String, String)> {
    let mut commits_by_revision = BTreeMap::<String, BTreeSet<String>>::new();
    for anchor in anchors.flatten() {
        if let Some(commit_oid) = &anchor.commit_oid {
            commits_by_revision
                .entry(anchor.revision_id.clone())
                .or_default()
                .insert(commit_oid.clone());
        }
    }
    if projection.access.can_read_private_files {
        return commits_by_revision
            .into_iter()
            .flat_map(|(revision_id, commit_oids)| {
                commit_oids
                    .into_iter()
                    .map(move |commit_oid| (revision_id.clone(), commit_oid))
            })
            .collect();
    }
    RequestRevisionCommitVisibility::new(
        projection.state,
        projection.owner,
        projection.repo_name,
        projection.repo,
        projection.access,
        projection.request,
    )
    .visible_commits(&commits_by_revision)
    .await
}

fn discussion_anchor_response(
    anchor: Option<scope_domain::requests::RequestDiscussionAnchor>,
    visible_commits: &BTreeSet<(String, String)>,
) -> Option<RequestDiscussionAnchor> {
    let anchor = anchor?;
    let commit_context_is_visible = match anchor.commit_oid.as_deref() {
        None => anchor.path.is_none(),
        Some(commit_oid) => {
            visible_commits.contains(&(anchor.revision_id.clone(), commit_oid.to_string()))
        }
    };
    Some(project_discussion_anchor(anchor, commit_context_is_visible))
}

fn project_discussion_anchor(
    anchor: scope_domain::requests::RequestDiscussionAnchor,
    commit_context_is_visible: bool,
) -> RequestDiscussionAnchor {
    RequestDiscussionAnchor {
        revision_id: anchor.revision_id,
        commit_oid: commit_context_is_visible
            .then_some(anchor.commit_oid)
            .flatten(),
        path: commit_context_is_visible
            .then_some(anchor.path)
            .flatten()
            .map(|path| path.as_str().to_string()),
    }
}

fn reply_response(
    reply: scope_domain::requests::RequestDiscussionReply,
    child_reply_count: u64,
    users: &BTreeMap<String, scope_domain::store::UserAccount>,
) -> Result<RequestDiscussionReplyResponse, ApiError> {
    Ok(RequestDiscussionReplyResponse {
        id: reply.id,
        discussion_id: reply.discussion_id,
        position: reply.position,
        author: request_actor_summary_response(&reply.author_user_id, users)?,
        body_markdown: reply.body_markdown,
        reply_to_reply_id: reply.reply_to_reply_id,
        child_reply_count,
        can_reply: reply.depth < scope_domain::requests::REQUEST_DISCUSSION_REPLY_MAX_DEPTH,
        created_at_unix: reply.created_at_unix,
    })
}

fn reply_read_response(
    model: scope_postgres::db::RequestDiscussionReplyReadModel,
    users: &BTreeMap<String, scope_domain::store::UserAccount>,
) -> Result<RequestDiscussionReplyResponse, ApiError> {
    reply_response(model.reply, model.child_reply_count, users)
}

#[derive(Debug)]
struct DiscussionCursor {
    snapshot_version: u64,
    position: u64,
    id: String,
}

fn parse_discussion_cursor(value: &str) -> Result<DiscussionCursor, ApiError> {
    let mut parts = value.splitn(3, ':');
    let snapshot_version = parts
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ApiError::bad_request("invalid discussion cursor"))?;
    let position = parts
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ApiError::bad_request("invalid discussion cursor"))?;
    let id = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("invalid discussion cursor"))?
        .to_string();
    Ok(DiscussionCursor {
        snapshot_version,
        position,
        id,
    })
}

fn encode_discussion_cursor(snapshot_version: u64, position: u64, id: &str) -> String {
    format!("{snapshot_version}:{position}:{id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::{
        policy::ScopePath, requests::RequestDiscussionAnchor as DomainDiscussionAnchor,
    };

    #[test]
    fn discussion_anchor_hides_whole_commit_context_when_policy_rejects_it() {
        let private_path = ScopePath::parse("/internal/plan.md").unwrap();
        let anchor = DomainDiscussionAnchor {
            revision_id: "revision".to_string(),
            commit_oid: Some("commit".to_string()),
            path: Some(private_path),
        };

        let public = project_discussion_anchor(anchor.clone(), false);
        let private = project_discussion_anchor(anchor, true);

        assert_eq!(public.path, None);
        assert_eq!(public.commit_oid, None);
        assert_eq!(private.path.as_deref(), Some("/internal/plan.md"));
        assert_eq!(private.commit_oid.as_deref(), Some("commit"));
    }

    #[test]
    fn discussion_anchor_hides_commit_only_context_from_public_readers() {
        let anchor = DomainDiscussionAnchor {
            revision_id: "revision".to_string(),
            commit_oid: Some("commit".to_string()),
            path: None,
        };

        let public = project_discussion_anchor(anchor, false);

        assert_eq!(public.commit_oid, None);
        assert_eq!(public.path, None);
    }
}
