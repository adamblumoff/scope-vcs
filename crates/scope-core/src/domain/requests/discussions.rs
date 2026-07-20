use super::{
    REQUEST_DISCUSSION_BODY_MAX_BYTES, REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES, Request,
    RequestEvent, RequestEventKind, RequestEventPayload, RequestState, validate_body_size,
    validate_required_body, validate_required_id,
};
use crate::error::ApiError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestDiscussionStatus {
    Dormant,
    Open,
    Resolved,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestDiscussionSubject {
    Comment,
    ChangeBlock { change_block_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDiscussion {
    pub id: String,
    pub request_id: String,
    pub opened_position: u64,
    pub last_activity_position: u64,
    pub author_user_id: String,
    pub subject: RequestDiscussionSubject,
    pub body_markdown: Option<String>,
    pub status: RequestDiscussionStatus,
    pub client_discussion_id: String,
    pub created_at_unix: u64,
    pub resolved_at_unix: Option<u64>,
    pub resolved_by_user_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDiscussionReply {
    pub id: String,
    pub discussion_id: String,
    pub position: u64,
    pub author_user_id: String,
    pub body_markdown: String,
    pub reply_to_reply_id: Option<String>,
    pub client_reply_id: String,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDiscussionReadState {
    pub discussion_id: String,
    pub user_id: String,
    pub read_through_position: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug)]
pub struct CreateRequestDiscussionInput {
    pub request_id: String,
    pub id: String,
    pub actor_user_id: String,
    pub actor_can_participate: bool,
    pub client_discussion_id: String,
    pub body_markdown: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateRequestDiscussionMutation {
    pub request: Request,
    pub discussion: RequestDiscussion,
    pub read_state: RequestDiscussionReadState,
}

#[derive(Clone, Debug)]
pub struct CreateRequestDiscussionReplyInput {
    pub request_id: String,
    pub discussion_id: String,
    pub id: String,
    pub actor_user_id: String,
    pub actor_can_participate: bool,
    pub client_reply_id: String,
    pub body_markdown: String,
    pub reply_to_reply_id: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateRequestDiscussionReplyMutation {
    pub request: Request,
    pub discussion: RequestDiscussion,
    pub reply: RequestDiscussionReply,
    pub read_state: RequestDiscussionReadState,
    pub activity_event: Option<RequestEvent>,
}

#[derive(Clone, Debug)]
pub struct ResolveRequestDiscussionInput {
    pub request_id: String,
    pub discussion_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct ReopenRequestDiscussionInput {
    pub request_id: String,
    pub discussion_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestDiscussionMutation {
    pub request: Request,
    pub discussion: RequestDiscussion,
    pub event: RequestEvent,
}

#[derive(Clone, Debug)]
pub struct ReopenAndReplyToRequestDiscussionInput {
    pub request_id: String,
    pub discussion_id: String,
    pub reply_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub actor_can_participate: bool,
    pub event_id: String,
    pub client_reply_id: String,
    pub body_markdown: String,
    pub reply_to_reply_id: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct MarkRequestDiscussionReadInput {
    pub discussion_id: String,
    pub user_id: String,
    pub through_position: u64,
    pub now_unix: u64,
}

struct DiscussionTransitionInput {
    request_id: String,
    discussion_id: String,
    actor_user_id: String,
    actor_is_maintainer: bool,
    event_id: String,
    target: RequestDiscussionStatus,
    now_unix: u64,
}

pub fn create_request_discussion(
    requests: &mut BTreeMap<String, Request>,
    discussions: &mut BTreeMap<String, RequestDiscussion>,
    input: CreateRequestDiscussionInput,
) -> Result<CreateRequestDiscussionMutation, ApiError> {
    validate_common(
        &input.request_id,
        &input.id,
        &input.actor_user_id,
        &input.client_discussion_id,
        &input.body_markdown,
    )?;
    if !input.actor_can_participate {
        return Err(ApiError::forbidden("request discussion access required"));
    }
    if discussions.contains_key(&input.id) {
        return Err(ApiError::conflict("request discussion already exists"));
    }
    let request = open_request_mut(requests, &input.request_id)?;
    let position = advance_activity(request)?;
    let discussion = RequestDiscussion {
        id: input.id,
        request_id: request.id.clone(),
        opened_position: position,
        last_activity_position: position,
        author_user_id: input.actor_user_id.clone(),
        subject: RequestDiscussionSubject::Comment,
        body_markdown: Some(input.body_markdown),
        status: RequestDiscussionStatus::Open,
        client_discussion_id: input.client_discussion_id,
        created_at_unix: input.now_unix,
        resolved_at_unix: None,
        resolved_by_user_id: None,
    };
    let read_state = read_state(&discussion, &input.actor_user_id, position, input.now_unix);
    discussions.insert(discussion.id.clone(), discussion.clone());
    Ok(CreateRequestDiscussionMutation {
        request: request.clone(),
        discussion,
        read_state,
    })
}

pub fn create_request_discussion_reply(
    requests: &mut BTreeMap<String, Request>,
    discussions: &mut BTreeMap<String, RequestDiscussion>,
    replies: &mut BTreeMap<String, RequestDiscussionReply>,
    input: CreateRequestDiscussionReplyInput,
) -> Result<CreateRequestDiscussionReplyMutation, ApiError> {
    validate_reply_input(
        &input.request_id,
        &input.discussion_id,
        &input.id,
        &input.actor_user_id,
        &input.client_reply_id,
        &input.body_markdown,
    )?;
    if !input.actor_can_participate {
        return Err(ApiError::forbidden("request discussion access required"));
    }
    if replies.contains_key(&input.id) {
        return Err(ApiError::conflict(
            "request discussion reply already exists",
        ));
    }
    validate_reply_target(
        discussions,
        replies,
        &input.discussion_id,
        input.reply_to_reply_id.as_deref(),
    )?;
    let request = open_request_mut(requests, &input.request_id)?;
    let discussion = discussion_mut(discussions, &input.request_id, &input.discussion_id)?;
    if discussion.status == RequestDiscussionStatus::Resolved {
        return Err(ApiError::conflict("request discussion is resolved"));
    }
    let position = advance_activity(request)?;
    if discussion.status == RequestDiscussionStatus::Dormant {
        discussion.status = RequestDiscussionStatus::Open;
    }
    discussion.last_activity_position = position;
    let reply = RequestDiscussionReply {
        id: input.id,
        discussion_id: discussion.id.clone(),
        position,
        author_user_id: input.actor_user_id.clone(),
        body_markdown: input.body_markdown,
        reply_to_reply_id: input.reply_to_reply_id,
        client_reply_id: input.client_reply_id,
        created_at_unix: input.now_unix,
    };
    let read_state = read_state(discussion, &input.actor_user_id, position, input.now_unix);
    replies.insert(reply.id.clone(), reply.clone());
    Ok(CreateRequestDiscussionReplyMutation {
        request: request.clone(),
        discussion: discussion.clone(),
        reply,
        read_state,
        activity_event: None,
    })
}

pub fn resolve_request_discussion(
    requests: &mut BTreeMap<String, Request>,
    discussions: &mut BTreeMap<String, RequestDiscussion>,
    input: ResolveRequestDiscussionInput,
) -> Result<RequestDiscussionMutation, ApiError> {
    transition_discussion(
        requests,
        discussions,
        DiscussionTransitionInput {
            request_id: input.request_id,
            discussion_id: input.discussion_id,
            actor_user_id: input.actor_user_id,
            actor_is_maintainer: input.actor_is_maintainer,
            event_id: input.event_id,
            target: RequestDiscussionStatus::Resolved,
            now_unix: input.now_unix,
        },
    )
}

pub fn reopen_request_discussion(
    requests: &mut BTreeMap<String, Request>,
    discussions: &mut BTreeMap<String, RequestDiscussion>,
    input: ReopenRequestDiscussionInput,
) -> Result<RequestDiscussionMutation, ApiError> {
    transition_discussion(
        requests,
        discussions,
        DiscussionTransitionInput {
            request_id: input.request_id,
            discussion_id: input.discussion_id,
            actor_user_id: input.actor_user_id,
            actor_is_maintainer: input.actor_is_maintainer,
            event_id: input.event_id,
            target: RequestDiscussionStatus::Open,
            now_unix: input.now_unix,
        },
    )
}

pub fn reopen_and_reply_to_request_discussion(
    requests: &mut BTreeMap<String, Request>,
    discussions: &mut BTreeMap<String, RequestDiscussion>,
    replies: &mut BTreeMap<String, RequestDiscussionReply>,
    input: ReopenAndReplyToRequestDiscussionInput,
) -> Result<CreateRequestDiscussionReplyMutation, ApiError> {
    if !input.actor_can_participate {
        return Err(ApiError::forbidden("request discussion access required"));
    }
    validate_reply_input(
        &input.request_id,
        &input.discussion_id,
        &input.reply_id,
        &input.actor_user_id,
        &input.client_reply_id,
        &input.body_markdown,
    )?;
    validate_required_id("event id", &input.event_id)?;
    validate_reply_target(
        discussions,
        replies,
        &input.discussion_id,
        input.reply_to_reply_id.as_deref(),
    )?;
    let request_author_user_id = open_request_mut(requests, &input.request_id)?
        .author_user_id
        .clone();
    let discussion = discussion_mut(discussions, &input.request_id, &input.discussion_id)?;
    ensure_can_transition(
        discussion,
        &request_author_user_id,
        &input.actor_user_id,
        input.actor_is_maintainer,
    )?;
    if discussion.status != RequestDiscussionStatus::Resolved {
        return Err(ApiError::conflict("request discussion is already open"));
    }
    let request = requests
        .get_mut(&input.request_id)
        .expect("validated request");
    let position = advance_activity(request)?;
    discussion.status = RequestDiscussionStatus::Open;
    discussion.resolved_at_unix = None;
    discussion.resolved_by_user_id = None;
    discussion.last_activity_position = position;
    let reply = RequestDiscussionReply {
        id: input.reply_id,
        discussion_id: discussion.id.clone(),
        position,
        author_user_id: input.actor_user_id.clone(),
        body_markdown: input.body_markdown,
        reply_to_reply_id: input.reply_to_reply_id,
        client_reply_id: input.client_reply_id,
        created_at_unix: input.now_unix,
    };
    let read_state = read_state(discussion, &input.actor_user_id, position, input.now_unix);
    let activity_event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::DiscussionReopened,
        position,
        payload: RequestEventPayload::DiscussionReopened {
            discussion_id: discussion.id.clone(),
        },
        created_at_unix: input.now_unix,
    };
    replies.insert(reply.id.clone(), reply.clone());
    Ok(CreateRequestDiscussionReplyMutation {
        request: request.clone(),
        discussion: discussion.clone(),
        reply,
        read_state,
        activity_event: Some(activity_event),
    })
}

pub fn mark_request_discussion_read(
    discussions: &BTreeMap<String, RequestDiscussion>,
    read_states: &mut BTreeMap<(String, String), RequestDiscussionReadState>,
    input: MarkRequestDiscussionReadInput,
) -> Result<RequestDiscussionReadState, ApiError> {
    validate_required_id("discussion id", &input.discussion_id)?;
    validate_required_id("user id", &input.user_id)?;
    let discussion = discussions
        .get(&input.discussion_id)
        .ok_or_else(|| ApiError::not_found("request discussion not found"))?;
    let through = input
        .through_position
        .min(discussion.last_activity_position);
    let key = (input.discussion_id.clone(), input.user_id.clone());
    let state = read_states
        .entry(key)
        .or_insert_with(|| RequestDiscussionReadState {
            discussion_id: input.discussion_id,
            user_id: input.user_id,
            read_through_position: 0,
            updated_at_unix: input.now_unix,
        });
    if through > state.read_through_position {
        state.read_through_position = through;
        state.updated_at_unix = input.now_unix;
    }
    Ok(state.clone())
}

fn transition_discussion(
    requests: &mut BTreeMap<String, Request>,
    discussions: &mut BTreeMap<String, RequestDiscussion>,
    input: DiscussionTransitionInput,
) -> Result<RequestDiscussionMutation, ApiError> {
    validate_required_id("event id", &input.event_id)?;
    let request_author_user_id = open_request_mut(requests, &input.request_id)?
        .author_user_id
        .clone();
    let discussion = discussion_mut(discussions, &input.request_id, &input.discussion_id)?;
    ensure_can_transition(
        discussion,
        &request_author_user_id,
        &input.actor_user_id,
        input.actor_is_maintainer,
    )?;
    if discussion.status == input.target {
        return Err(ApiError::conflict(match input.target {
            RequestDiscussionStatus::Dormant => "request discussion is already dormant",
            RequestDiscussionStatus::Open => "request discussion is already open",
            RequestDiscussionStatus::Resolved => "request discussion is already resolved",
        }));
    }
    if discussion.status == RequestDiscussionStatus::Dormant {
        return Err(ApiError::conflict("request discussion has no comments"));
    }
    let request = requests
        .get_mut(&input.request_id)
        .expect("validated request");
    let position = advance_activity(request)?;
    discussion.status = input.target;
    discussion.last_activity_position = position;
    let (kind, payload) = match input.target {
        RequestDiscussionStatus::Dormant => unreachable!("dormant is not a transition target"),
        RequestDiscussionStatus::Open => {
            discussion.resolved_at_unix = None;
            discussion.resolved_by_user_id = None;
            (
                RequestEventKind::DiscussionReopened,
                RequestEventPayload::DiscussionReopened {
                    discussion_id: discussion.id.clone(),
                },
            )
        }
        RequestDiscussionStatus::Resolved => {
            discussion.resolved_at_unix = Some(input.now_unix);
            discussion.resolved_by_user_id = Some(input.actor_user_id.clone());
            (
                RequestEventKind::DiscussionResolved,
                RequestEventPayload::DiscussionResolved {
                    discussion_id: discussion.id.clone(),
                },
            )
        }
    };
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind,
        position,
        payload,
        created_at_unix: input.now_unix,
    };
    Ok(RequestDiscussionMutation {
        request: request.clone(),
        discussion: discussion.clone(),
        event,
    })
}

fn ensure_can_transition(
    discussion: &RequestDiscussion,
    request_author_user_id: &str,
    actor_user_id: &str,
    actor_is_maintainer: bool,
) -> Result<(), ApiError> {
    if actor_is_maintainer
        || discussion.author_user_id == actor_user_id
        || request_author_user_id == actor_user_id
    {
        Ok(())
    } else {
        Err(ApiError::forbidden(
            "request discussion resolution access required",
        ))
    }
}

fn validate_common(
    request_id: &str,
    id: &str,
    actor: &str,
    client_id: &str,
    body: &str,
) -> Result<(), ApiError> {
    validate_required_id("request id", request_id)?;
    validate_required_id("discussion id", id)?;
    validate_required_id("actor user id", actor)?;
    validate_required_id("client discussion id", client_id)?;
    validate_body_size(
        "client discussion id",
        client_id,
        REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES,
    )?;
    validate_required_body("discussion body", body)?;
    validate_body_size("discussion body", body, REQUEST_DISCUSSION_BODY_MAX_BYTES)
}

fn validate_reply_input(
    request_id: &str,
    discussion_id: &str,
    id: &str,
    actor: &str,
    client_id: &str,
    body: &str,
) -> Result<(), ApiError> {
    validate_required_id("request id", request_id)?;
    validate_required_id("discussion id", discussion_id)?;
    validate_required_id("reply id", id)?;
    validate_required_id("actor user id", actor)?;
    validate_required_id("client reply id", client_id)?;
    validate_body_size(
        "client reply id",
        client_id,
        REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES,
    )?;
    validate_required_body("reply body", body)?;
    validate_body_size("reply body", body, REQUEST_DISCUSSION_BODY_MAX_BYTES)
}

fn validate_reply_target(
    discussions: &BTreeMap<String, RequestDiscussion>,
    replies: &BTreeMap<String, RequestDiscussionReply>,
    discussion_id: &str,
    reply_to: Option<&str>,
) -> Result<(), ApiError> {
    if !discussions.contains_key(discussion_id) {
        return Err(ApiError::not_found("request discussion not found"));
    }
    if let Some(reply_id) = reply_to {
        let reply = replies
            .get(reply_id)
            .ok_or_else(|| ApiError::bad_request("quoted reply not found"))?;
        if reply.discussion_id != discussion_id {
            return Err(ApiError::bad_request(
                "quoted reply belongs to another discussion",
            ));
        }
    }
    Ok(())
}

fn discussion_mut<'a>(
    discussions: &'a mut BTreeMap<String, RequestDiscussion>,
    request_id: &str,
    discussion_id: &str,
) -> Result<&'a mut RequestDiscussion, ApiError> {
    discussions
        .get_mut(discussion_id)
        .filter(|discussion| discussion.request_id == request_id)
        .ok_or_else(|| ApiError::not_found("request discussion not found"))
}

fn open_request_mut<'a>(
    requests: &'a mut BTreeMap<String, Request>,
    request_id: &str,
) -> Result<&'a mut Request, ApiError> {
    let request = requests
        .get_mut(request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if matches!(
        request.state,
        RequestState::Resolved | RequestState::Withdrawn
    ) {
        return Err(ApiError::conflict("request is closed"));
    }
    Ok(request)
}

fn advance_activity(request: &mut Request) -> Result<u64, ApiError> {
    request.activity_version = request
        .activity_version
        .checked_add(1)
        .ok_or_else(|| ApiError::conflict("request activity version overflow"))?;
    Ok(request.activity_version)
}

fn read_state(
    discussion: &RequestDiscussion,
    user_id: &str,
    position: u64,
    now_unix: u64,
) -> RequestDiscussionReadState {
    RequestDiscussionReadState {
        discussion_id: discussion.id.clone(),
        user_id: user_id.to_string(),
        read_through_position: position,
        updated_at_unix: now_unix,
    }
}
