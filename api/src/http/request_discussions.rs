use crate::{
    auth::scope::require_scope_user,
    error::ApiError,
    http::{request_review::RequestRevisionCommitVisibility, requests::*, responses::*},
    state::AppState,
    use_cases::request_discussion_mutation::{
        self, CreateDiscussionCommand, CreateReplyCommand, DiscussionAnchorInput,
        DiscussionMutationResult, DiscussionTransition, MarkDiscussionReadCommand,
        ReopenAndReplyCommand, ReplyMutationResult, TransitionDiscussionCommand,
    },
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
    RequestDiscussionReplyReferenceResponse, RequestDiscussionReplyResponse,
    RequestDiscussionSummaryResponse,
};
use scope_domain::requests::{REQUEST_ACTIVITY_PAGE_MAX_EVENTS, RequestViewer, request_policy};
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
    reply: Option<String>,
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
    let result = request_discussion_mutation::create_discussion(
        &state,
        CreateDiscussionCommand {
            owner,
            repo_name,
            request_id,
            actor_user_id: user.id,
            client_discussion_id: input.client_discussion_id,
            body_markdown: input.body_markdown,
            anchor: input.anchor.map(|anchor| DiscussionAnchorInput {
                revision_id: anchor.revision_id,
                commit_oid: anchor.commit_oid,
                path: anchor.path,
            }),
        },
    )
    .await?;
    let discussion = mutation_discussion_response(result)?;
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
    if let Some(reply_id) = query.reply.as_deref() {
        if query.before.is_some() {
            return Err(ApiError::bad_request(
                "reply and before cannot be used together",
            ));
        }
        let Some((reply, users)) = state
            .metadata
            .requests()
            .request_discussion_reply(&discussion_id, reply_id)
            .await?
        else {
            return Ok(Json(RequestDiscussionRepliesPageResponse {
                replies: Vec::new(),
                next_before_position: None,
            }));
        };
        return Ok(Json(RequestDiscussionRepliesPageResponse {
            replies: vec![reply_response(reply, &users)?],
            next_before_position: None,
        }));
    }
    let limit = query
        .limit
        .unwrap_or(DEFAULT_REPLY_LIMIT)
        .clamp(1, MAX_REPLY_LIMIT);
    let (mut replies, users) = state
        .metadata
        .requests()
        .request_discussion_replies(&discussion_id, query.before, limit + 1)
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
        .map(|reply| reply_response(reply, &users))
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
    let result = request_discussion_mutation::create_reply(
        &state,
        CreateReplyCommand {
            owner,
            repo_name,
            request_id,
            discussion_id,
            actor_user_id: user.id,
            client_reply_id: input.client_reply_id,
            body_markdown: input.body_markdown,
            reply_to_reply_id: input.reply_to_reply_id,
        },
    )
    .await?;
    Ok(Json(reply_mutation_response(result)?))
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
        DiscussionTransition::Resolve,
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
        DiscussionTransition::Reopen,
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
    let result = request_discussion_mutation::reopen_and_reply(
        &state,
        ReopenAndReplyCommand {
            owner,
            repo_name,
            request_id,
            discussion_id,
            actor_user_id: user.id,
            client_reply_id: input.client_reply_id,
            body_markdown: input.body_markdown,
            reply_to_reply_id: input.reply_to_reply_id,
        },
    )
    .await?;
    Ok(Json(reply_mutation_response(result)?))
}

pub(crate) async fn mark_read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id, discussion_id)): Path<(String, String, String, String)>,
    Json(input): Json<MarkRequestDiscussionReadRequest>,
) -> Result<Json<RequestDiscussionReadResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let result = request_discussion_mutation::mark_read(
        &state,
        MarkDiscussionReadCommand {
            owner,
            repo_name,
            request_id,
            discussion_id,
            actor_user_id: user.id,
            through_position: input.through_position,
        },
    )
    .await?;
    Ok(Json(RequestDiscussionReadResponse {
        read_through_position: result.read_through_position,
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
    transition: DiscussionTransition,
) -> Result<Json<RequestDiscussionMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let result = request_discussion_mutation::transition_discussion(
        &state,
        TransitionDiscussionCommand {
            owner,
            repo_name,
            request_id,
            discussion_id,
            actor_user_id: user.id,
            transition,
        },
    )
    .await?;
    let discussion = mutation_discussion_response(result)?;
    Ok(Json(RequestDiscussionMutationResponse { discussion }))
}

fn mutation_discussion_response(
    result: DiscussionMutationResult,
) -> Result<RequestDiscussionSummaryResponse, ApiError> {
    let anchor = discussion_anchor_response(
        result.discussion.discussion.anchor.clone(),
        &result.visible_anchor_commits,
        result.discussion.anchor_revision_position,
    )?;
    discussion_summary(result.discussion, &result.users, anchor)
}

fn reply_mutation_response(
    result: ReplyMutationResult,
) -> Result<RequestDiscussionReplyMutationResponse, ApiError> {
    let discussion = mutation_discussion_response(result.discussion)?;
    let reply = reply_response(result.reply, &result.reply_users)?;
    Ok(RequestDiscussionReplyMutationResponse { discussion, reply })
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
    users: &BTreeMap<String, scope_domain::account::UserAccount>,
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
        read_through_position: model.read_through_position,
        unread_count: model.unread_count,
        latest_replies: model
            .latest_replies
            .into_iter()
            .map(|reply| reply_response(reply, users))
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
    repo: &'a scope_domain::repository::Repository,
    access: scope_domain::repository::access::RepositoryAccess,
}

async fn discussion_summaries(
    projection: &DiscussionProjection<'_>,
    models: Vec<scope_postgres::db::RequestDiscussionReadModel>,
    users: &BTreeMap<String, scope_domain::account::UserAccount>,
) -> Result<Vec<RequestDiscussionSummaryResponse>, ApiError> {
    let visibility = discussion_anchor_visibility(
        projection,
        models.iter().map(|model| model.discussion.anchor.as_ref()),
    )
    .await;
    let mut summaries = Vec::with_capacity(models.len());
    for model in models {
        let anchor = discussion_anchor_response(
            model.discussion.anchor.clone(),
            &visibility,
            model.anchor_revision_position,
        )?;
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
    revision_position: Option<u64>,
) -> Result<Option<RequestDiscussionAnchor>, ApiError> {
    let Some(anchor) = anchor else {
        return Ok(None);
    };
    let commit_context_is_visible = match anchor.commit_oid.as_deref() {
        None => anchor.path.is_none(),
        Some(commit_oid) => {
            visible_commits.contains(&(anchor.revision_id.clone(), commit_oid.to_string()))
        }
    };
    let revision_position = revision_position.ok_or_else(|| {
        ApiError::internal_message(format!(
            "request discussion anchor references unknown revision {}",
            anchor.revision_id
        ))
    })?;
    Ok(Some(project_discussion_anchor(
        anchor,
        revision_position,
        commit_context_is_visible,
    )))
}

fn project_discussion_anchor(
    anchor: scope_domain::requests::RequestDiscussionAnchor,
    revision_position: u64,
    commit_context_is_visible: bool,
) -> RequestDiscussionAnchor {
    RequestDiscussionAnchor {
        revision_id: anchor.revision_id,
        revision_position,
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
    model: scope_postgres::db::RequestDiscussionReplyReadModel,
    users: &BTreeMap<String, scope_domain::account::UserAccount>,
) -> Result<RequestDiscussionReplyResponse, ApiError> {
    let reply_to = model
        .reply_to
        .map(|target| {
            Ok::<_, ApiError>(RequestDiscussionReplyReferenceResponse {
                id: target.id,
                position: target.position,
                author: request_actor_summary_response(&target.author_user_id, users)?,
                body_markdown: target.body_markdown,
            })
        })
        .transpose()?;
    let reply = model.reply;
    Ok(RequestDiscussionReplyResponse {
        id: reply.id,
        discussion_id: reply.discussion_id,
        position: reply.position,
        author: request_actor_summary_response(&reply.author_user_id, users)?,
        body_markdown: reply.body_markdown,
        reply_to,
        created_at_unix: reply.created_at_unix,
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

        let public = project_discussion_anchor(anchor.clone(), 3, false);
        let private = project_discussion_anchor(anchor, 3, true);

        assert_eq!(public.path, None);
        assert_eq!(public.commit_oid, None);
        assert_eq!(private.path.as_deref(), Some("/internal/plan.md"));
        assert_eq!(private.commit_oid.as_deref(), Some("commit"));
    }

    #[test]
    fn discussion_anchor_carries_the_revision_ordinal() {
        let anchor = DomainDiscussionAnchor {
            revision_id: "revision".to_string(),
            commit_oid: None,
            path: None,
        };
        let response = discussion_anchor_response(Some(anchor), &BTreeSet::new(), Some(4))
            .unwrap()
            .unwrap();

        assert_eq!(response.revision_id, "revision");
        assert_eq!(response.revision_position, 4);
    }

    #[test]
    fn discussion_anchor_rejects_an_unknown_revision() {
        let anchor = DomainDiscussionAnchor {
            revision_id: "missing".to_string(),
            commit_oid: None,
            path: None,
        };

        let error = discussion_anchor_response(Some(anchor), &BTreeSet::new(), None).unwrap_err();

        assert!(format!("{error:?}").contains("unknown revision"));
    }

    #[test]
    fn discussion_anchor_hides_commit_only_context_from_public_readers() {
        let anchor = DomainDiscussionAnchor {
            revision_id: "revision".to_string(),
            commit_oid: Some("commit".to_string()),
            path: None,
        };

        let public = project_discussion_anchor(anchor, 1, false);

        assert_eq!(public.commit_oid, None);
        assert_eq!(public.path, None);
    }
}
