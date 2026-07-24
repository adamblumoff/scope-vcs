use super::{
    REQUEST_TITLE_MAX_BYTES, Request, RequestActorRole, RequestAudience, RequestChangeBlock,
    RequestDiscussion, RequestDiscussionReadState, RequestEvent, RequestEventKind,
    RequestEventPayload, RequestState, advance_request_activity, ensure_event_id_available,
    validate_body_size, validate_required_id,
};
use crate::{domain::store::SourceBlob, error::ApiError};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct StartRequestInput {
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub author_user_id: String,
    pub title: Option<String>,
    pub author_role: RequestActorRole,
    pub audience: RequestAudience,
    pub base_main_oid: String,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartRequestMutation {
    pub request: Request,
    pub event: RequestEvent,
}

#[derive(Clone, Debug)]
pub struct RecordWorkingRequestUploadInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_can_edit: bool,
    pub expected_old_head_oid: Option<String>,
    pub new_head_oid: String,
    pub git_snapshot: SourceBlob,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkingRequestUploadMutation {
    pub request: Request,
    pub orphan_objects: Vec<SourceBlob>,
}

#[derive(Clone, Debug)]
pub struct RecordRequestRevisionInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_can_edit: bool,
    pub expected_old_head_oid: Option<String>,
    pub new_head_oid: String,
    pub git_snapshot: SourceBlob,
    pub event_id: String,
    pub body: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestRevisionMutation {
    pub request: Request,
    pub event: RequestEvent,
    pub change_block: RequestChangeBlock,
    pub discussion: RequestDiscussion,
    pub read_state: RequestDiscussionReadState,
    pub orphan_objects: Vec<SourceBlob>,
}

#[derive(Clone, Debug)]
pub struct CloseRequestInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_can_close: bool,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseRequestMutation {
    DeletedDraft {
        request: Request,
        events: Vec<RequestEvent>,
        change_blocks: Vec<RequestChangeBlock>,
        orphan_objects: Vec<SourceBlob>,
    },
    Completed {
        request: Request,
        event: RequestEvent,
    },
}

pub fn start_request(
    requests: &mut BTreeMap<String, Request>,
    input: StartRequestInput,
) -> Result<StartRequestMutation, ApiError> {
    validate_start_request_input(&input)?;
    if requests.contains_key(&input.id) {
        return Err(ApiError::conflict("request already exists"));
    }
    ensure_request_name_available(requests, &input.repo_id, &input.name)?;
    let title = input.title.unwrap_or_else(|| input.name.clone());
    let request = Request {
        id: input.id,
        repo_id: input.repo_id,
        name: input.name,
        author_user_id: input.author_user_id,
        author_role: input.author_role,
        audience: input.audience,
        base_main_oid: input.base_main_oid.clone(),
        head_oid: input.base_main_oid,
        git_snapshot: None,
        title,
        description_markdown: String::new(),
        state: RequestState::Working,
        activity_version: 1,
        current_stake_credits: 0,
        first_ready_at_unix: None,
        ready_at_unix: None,
        held_at_unix: None,
        held_by_user_id: None,
        assessment_outcome: None,
        assessment_body_markdown: None,
        assessed_at_unix: None,
        assessed_by_user_id: None,
        completed_at_unix: None,
        completed_by_user_id: None,
        merged_at_unix: None,
        merged_by_user_id: None,
        merged_head_oid: None,
        merged_main_oid: None,
        created_at_unix: input.now_unix,
        updated_at_unix: input.now_unix,
    };
    request.validate_facts()?;
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: request.author_user_id.clone(),
        kind: RequestEventKind::Started,
        position: 1,
        payload: RequestEventPayload::Started {
            title: request.title.clone(),
            description_markdown: request.description_markdown.clone(),
        },
        created_at_unix: input.now_unix,
    };
    requests.insert(request.id.clone(), request.clone());
    Ok(StartRequestMutation { request, event })
}

pub fn record_working_request_upload(
    requests: &mut BTreeMap<String, Request>,
    input: RecordWorkingRequestUploadInput,
) -> Result<WorkingRequestUploadMutation, ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("head oid", &input.new_head_oid)?;
    let request = requests
        .get_mut(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if !input.actor_can_edit {
        return Err(ApiError::forbidden("request branch edit access required"));
    }
    if request.state != RequestState::Working {
        return Err(ApiError::conflict("request is not working"));
    }
    validate_expected_head(request, input.expected_old_head_oid.as_deref())?;
    validate_snapshot_head(&input.git_snapshot, &input.new_head_oid)?;
    let old_git_snapshot = request.git_snapshot.replace(input.git_snapshot);
    request.head_oid = input.new_head_oid;
    request.updated_at_unix = input.now_unix;
    request.validate_facts()?;
    Ok(WorkingRequestUploadMutation {
        request: request.clone(),
        orphan_objects: old_git_snapshot.into_iter().collect(),
    })
}

pub fn record_request_revision(
    requests: &mut BTreeMap<String, Request>,
    events: &mut BTreeMap<String, RequestEvent>,
    input: RecordRequestRevisionInput,
) -> Result<RequestRevisionMutation, ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("head oid", &input.new_head_oid)?;
    validate_required_id("event id", &input.event_id)?;
    ensure_event_id_available(events, &input.event_id)?;
    let request = requests
        .get_mut(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if !input.actor_can_edit {
        return Err(ApiError::forbidden("request branch edit access required"));
    }
    if request.state != RequestState::Working {
        return Err(ApiError::conflict(
            "only working requests can receive new revisions",
        ));
    }
    validate_expected_head(request, input.expected_old_head_oid.as_deref())?;
    validate_snapshot_head(&input.git_snapshot, &input.new_head_oid)?;
    let old_head_oid = request.head_oid.clone();
    request.head_oid = input.new_head_oid.clone();
    let old_git_snapshot = request.git_snapshot.replace(input.git_snapshot.clone());
    request.updated_at_unix = input.now_unix;
    let position = advance_request_activity(request)?;
    request.validate_facts()?;
    let request = request.clone();
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::RevisionPushed,
        position,
        payload: RequestEventPayload::RevisionPushed {
            old_head_oid: old_head_oid.clone(),
            new_head_oid: input.new_head_oid.clone(),
            note: input.body,
        },
        created_at_unix: input.now_unix,
    };
    let (change_block, discussion, read_state) = super::change_blocks::revision_change_block(
        &request,
        &event,
        old_head_oid,
        input.new_head_oid,
    )?;
    events.insert(event.id.clone(), event.clone());
    Ok(RequestRevisionMutation {
        request,
        event,
        change_block,
        discussion,
        read_state,
        orphan_objects: old_git_snapshot.into_iter().collect(),
    })
}

pub fn close_request(
    requests: &mut BTreeMap<String, Request>,
    events: &mut BTreeMap<String, RequestEvent>,
    change_blocks: &mut BTreeMap<String, RequestChangeBlock>,
    input: CloseRequestInput,
) -> Result<CloseRequestMutation, ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("event id", &input.event_id)?;
    if !input.actor_can_close {
        return Err(ApiError::forbidden("request close access required"));
    }
    let request = requests
        .get(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if request.state != RequestState::Working {
        return Err(ApiError::conflict("only working requests can be closed"));
    }
    if !request.is_published() {
        let request = requests
            .remove(&input.request_id)
            .ok_or_else(|| ApiError::not_found("request not found"))?;
        let event_ids = events
            .values()
            .filter(|event| event.request_id == request.id)
            .map(|event| event.id.clone())
            .collect::<Vec<_>>();
        let removed_events = event_ids
            .into_iter()
            .filter_map(|event_id| events.remove(&event_id))
            .collect::<Vec<_>>();
        let change_block_ids = change_blocks
            .values()
            .filter(|change_block| change_block.request_id == request.id)
            .map(|change_block| change_block.id.clone())
            .collect::<Vec<_>>();
        let removed_change_blocks = change_block_ids
            .into_iter()
            .filter_map(|change_block_id| change_blocks.remove(&change_block_id))
            .collect::<Vec<_>>();
        let mut orphan_objects = request
            .git_snapshot
            .clone()
            .into_iter()
            .chain(
                removed_change_blocks
                    .iter()
                    .map(|change_block| change_block.git_snapshot.clone()),
            )
            .collect::<Vec<_>>();
        orphan_objects.sort_by(|left, right| left.object_key.cmp(&right.object_key));
        orphan_objects.dedup_by(|left, right| left.object_key == right.object_key);
        return Ok(CloseRequestMutation::DeletedDraft {
            request,
            events: removed_events,
            change_blocks: removed_change_blocks,
            orphan_objects,
        });
    }
    ensure_event_id_available(events, &input.event_id)?;
    let request = requests
        .get_mut(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    request.state = RequestState::Completed;
    request.completed_at_unix = Some(input.now_unix);
    request.completed_by_user_id = Some(input.actor_user_id.clone());
    request.updated_at_unix = input.now_unix;
    let position = advance_request_activity(request)?;
    request.validate_facts()?;
    let request = request.clone();
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::Closed,
        position,
        payload: RequestEventPayload::Closed {
            head_oid: request.head_oid.clone(),
        },
        created_at_unix: input.now_unix,
    };
    events.insert(event.id.clone(), event.clone());
    Ok(CloseRequestMutation::Completed { request, event })
}

fn validate_start_request_input(input: &StartRequestInput) -> Result<(), ApiError> {
    validate_required_id("request id", &input.id)?;
    validate_required_id("repo id", &input.repo_id)?;
    validate_required_id("author user id", &input.author_user_id)?;
    validate_request_name(&input.name)?;
    if let Some(title) = &input.title {
        validate_required_id("title", title)?;
        validate_body_size("request title", title, REQUEST_TITLE_MAX_BYTES)?;
    }
    validate_required_id("base main oid", &input.base_main_oid)?;
    validate_required_id("event id", &input.event_id)?;
    if input.author_role == RequestActorRole::Public && input.audience != RequestAudience::Public {
        return Err(ApiError::bad_request(
            "public contributors can only create public requests",
        ));
    }
    Ok(())
}

fn validate_expected_head(request: &Request, expected: Option<&str>) -> Result<(), ApiError> {
    match expected {
        Some(expected) if request.head_oid != expected => Err(ApiError::conflict(
            "request branch changed since push started; fetch and retry",
        )),
        None if request.git_snapshot.is_some() => Err(ApiError::conflict(
            "request branch changed since push started; fetch and retry",
        )),
        _ => Ok(()),
    }
}

fn validate_snapshot_head(snapshot: &SourceBlob, head_oid: &str) -> Result<(), ApiError> {
    if snapshot.git_oid == head_oid {
        Ok(())
    } else {
        Err(ApiError::conflict(
            "request revision snapshot does not match the new head",
        ))
    }
}

fn ensure_request_name_available(
    requests: &BTreeMap<String, Request>,
    repo_id: &str,
    request_name: &str,
) -> Result<(), ApiError> {
    if requests
        .values()
        .any(|request| request.repo_id == repo_id && request.name == request_name)
    {
        Err(ApiError::conflict("request name already exists"))
    } else {
        Ok(())
    }
}

pub fn validate_request_name(name: &str) -> Result<(), ApiError> {
    validate_required_id("request name", name)?;
    if name.len() > 48
        || !name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (index > 0 && byte == b'-')
        })
    {
        return Err(ApiError::bad_request(
            "request name must match [a-z0-9][a-z0-9-]{0,47}",
        ));
    }
    if matches!(name, "main" | "head" | "scope") {
        return Err(ApiError::bad_request("request name is reserved"));
    }
    Ok(())
}
