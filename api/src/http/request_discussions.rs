use crate::{
    auth::scope::require_scope_user,
    domain::requests::{
        CreateRequestDiscussionInput, CreateRequestDiscussionReplyInput,
        MarkRequestDiscussionReadInput, REQUEST_ACTIVITY_PAGE_MAX_EVENTS,
        ReopenAndReplyToRequestDiscussionInput,
    },
    error::ApiError,
    http::{requests::*, responses::*},
    persistence::unix_now,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use scope_api_contract::*;
use serde::Deserialize;
use std::collections::BTreeMap;

const DEFAULT_DISCUSSION_LIMIT: usize = 25;
const MAX_DISCUSSION_LIMIT: usize = 100;
const DEFAULT_REPLY_LIMIT: u64 = 50;
const MAX_REPLY_LIMIT: u64 = 100;

#[derive(Debug, Deserialize)]
pub(crate) struct DiscussionListQuery {
    cursor: Option<String>,
    limit: Option<usize>,
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
    let snapshot_version = cursor
        .as_ref()
        .map(|cursor| cursor.snapshot_version)
        .unwrap_or(request.activity_version);
    let batch = state
        .metadata
        .request_discussions_page(scope_core::db::RequestDiscussionsPageQuery {
            request_id: &request.id,
            viewer_user_id: viewer_user_id.as_deref(),
            snapshot_version,
            cursor: cursor
                .as_ref()
                .map(|cursor| (cursor.position, cursor.id.clone())),
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
    let discussions = discussions
        .into_iter()
        .map(|model| discussion_summary(model, &batch.users))
        .collect::<Result<Vec<_>, _>>()?;
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
    let mutation = state
        .metadata
        .create_request_discussion(CreateRequestDiscussionInput {
            request_id: request.id.clone(),
            id: random_id("discussion")?,
            actor_user_id: actor_user_id.clone(),
            actor_can_participate: false,
            client_discussion_id: input.client_discussion_id,
            body_markdown: input.body_markdown,
            now_unix: unix_now()?,
        })
        .await?;
    let through_position = mutation.discussion.last_activity_position;
    let discussion_id = mutation.discussion.id.clone();
    let discussion =
        load_one_summary(&state, &request.id, &discussion_id, Some(&actor_user_id)).await?;
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
    reply_mutation_response(
        &state,
        &repo,
        &request,
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
        .reopen_and_reply_to_request_discussion(ReopenAndReplyToRequestDiscussionInput {
            request_id: request.id.clone(),
            discussion_id: discussion_id.clone(),
            reply_id: random_id("discussion_reply")?,
            actor_user_id: actor_user_id.clone(),
            actor_is_maintainer: false,
            actor_can_participate: false,
            event_id: random_id("event_request_discussion_reopened")?,
            client_reply_id: input.client_reply_id,
            body_markdown: input.body_markdown,
            reply_to_reply_id: input.reply_to_reply_id,
            now_unix: unix_now()?,
        })
        .await?;
    reply_mutation_response(
        &state,
        &repo,
        &request,
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
    let discussions = batch
        .discussions
        .into_iter()
        .map(|model| discussion_summary(model, &batch.users))
        .collect::<Result<Vec<_>, _>>()?;
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
    let latest = query.latest.unwrap_or(false);
    let limit = query
        .limit
        .unwrap_or(REQUEST_ACTIVITY_PAGE_MAX_EVENTS)
        .clamp(1, REQUEST_ACTIVITY_PAGE_MAX_EVENTS);
    let events = if latest {
        state
            .metadata
            .latest_request_events(&request.id, limit as u64)
            .await?
    } else {
        state
            .metadata
            .request_events_after_position(&request.id, query.after.unwrap_or(0), limit as u64)
            .await?
    };
    let users = state
        .metadata
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
            let actor = actor_response(&event.actor_user_id, &users)?;
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
            .reopen_request_discussion(
                request.id.clone(),
                discussion_id.clone(),
                actor_user_id.clone(),
                random_id("event_request_discussion_reopened")?,
                unix_now()?,
            )
            .await?
    };
    let through_position = discussion.last_activity_position;
    let discussion =
        load_one_summary(&state, &request.id, &discussion_id, Some(&actor_user_id)).await?;
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
    state: &AppState,
    repo: &crate::domain::store::StoredRepository,
    request: &crate::domain::requests::Request,
    discussion_id: String,
    reply: crate::domain::requests::RequestDiscussionReply,
    actor_user_id: &str,
) -> Result<Json<RequestDiscussionReplyMutationResponse>, ApiError> {
    let discussion =
        load_one_summary(state, &request.id, &discussion_id, Some(actor_user_id)).await?;
    let users = state
        .metadata
        .users_by_ids([reply.author_user_id.clone()])
        .await?;
    let child_reply_count = state
        .metadata
        .request_discussion_reply_child_count(&reply.id)
        .await?;
    let response = reply_response(reply.clone(), child_reply_count, &users)?;
    state
        .publish_request_timeline_change(
            &repo.record.id,
            request.id.clone(),
            discussion_id,
            reply.position,
            request.audience,
        )
        .await;
    Ok(Json(RequestDiscussionReplyMutationResponse {
        discussion,
        reply: response,
    }))
}

async fn load_one_summary(
    state: &AppState,
    request_id: &str,
    discussion_id: &str,
    viewer_user_id: Option<&str>,
) -> Result<RequestDiscussionSummaryResponse, ApiError> {
    let (model, users) = state
        .metadata
        .request_discussion(request_id, discussion_id, viewer_user_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request discussion not found"))?;
    discussion_summary(model, &users)
}

async fn ensure_discussion_in_request(
    state: &AppState,
    request_id: &str,
    discussion_id: &str,
) -> Result<(), ApiError> {
    load_one_summary(state, request_id, discussion_id, None)
        .await
        .map(|_| ())
}

fn discussion_summary(
    model: crate::db::RequestDiscussionReadModel,
    users: &BTreeMap<String, crate::domain::store::UserAccount>,
) -> Result<RequestDiscussionSummaryResponse, ApiError> {
    Ok(RequestDiscussionSummaryResponse {
        id: model.discussion.id,
        request_id: model.discussion.request_id,
        client_discussion_id: model.discussion.client_discussion_id,
        opened_position: model.discussion.opened_position,
        last_activity_position: model.discussion.last_activity_position,
        author: actor_response(&model.discussion.author_user_id, users)?,
        body_markdown: model.discussion.body_markdown,
        change_block: model.change_block.map(|block| RequestChangeBlockResponse {
            id: block.id,
            position: block.position,
            old_head_oid: block.old_head_oid,
            new_head_oid: block.new_head_oid,
            created_at_unix: block.created_at_unix,
        }),
        status: model.discussion.status,
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
            .map(|id| actor_response(id, users))
            .transpose()?,
    })
}

fn reply_response(
    reply: crate::domain::requests::RequestDiscussionReply,
    child_reply_count: u64,
    users: &BTreeMap<String, crate::domain::store::UserAccount>,
) -> Result<RequestDiscussionReplyResponse, ApiError> {
    Ok(RequestDiscussionReplyResponse {
        id: reply.id,
        discussion_id: reply.discussion_id,
        position: reply.position,
        author: actor_response(&reply.author_user_id, users)?,
        body_markdown: reply.body_markdown,
        reply_to_reply_id: reply.reply_to_reply_id,
        child_reply_count,
        can_reply: reply.depth < crate::domain::requests::REQUEST_DISCUSSION_REPLY_MAX_DEPTH,
        created_at_unix: reply.created_at_unix,
    })
}

fn reply_read_response(
    model: scope_core::db::RequestDiscussionReplyReadModel,
    users: &BTreeMap<String, crate::domain::store::UserAccount>,
) -> Result<RequestDiscussionReplyResponse, ApiError> {
    reply_response(model.reply, model.child_reply_count, users)
}

fn actor_response(
    user_id: &str,
    users: &BTreeMap<String, crate::domain::store::UserAccount>,
) -> Result<RequestActorSummaryResponse, ApiError> {
    let user = users
        .get(user_id)
        .ok_or_else(|| ApiError::internal_message("request actor was not persisted"))?;
    Ok(RequestActorSummaryResponse {
        id: user.id.clone(),
        handle: user.handle.clone(),
    })
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
