use super::{Request, RequestEvent};
use crate::{error::DomainError, store::SourceBlob};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestRevision {
    pub id: String,
    pub request_id: String,
    pub position: u64,
    pub actor_user_id: String,
    pub old_head_oid: String,
    pub new_head_oid: String,
    pub git_snapshot: SourceBlob,
    pub created_at_unix: u64,
}

pub fn select_request_review_revision<'a>(
    revisions: &'a [RequestRevision],
    pinned_revision_id: Option<&str>,
) -> Result<Option<&'a RequestRevision>, DomainError> {
    if let Some(pinned_revision_id) = pinned_revision_id {
        return revisions
            .iter()
            .find(|revision| revision.id == pinned_revision_id)
            .map(Some)
            .ok_or_else(|| DomainError::not_found("request revision not found"));
    }
    Ok(revisions.iter().max_by(|left, right| {
        left.position
            .cmp(&right.position)
            .then_with(|| left.id.cmp(&right.id))
    }))
}

pub(super) fn revision(
    request: &Request,
    event: &RequestEvent,
    old_head_oid: String,
    new_head_oid: String,
) -> Result<RequestRevision, DomainError> {
    let git_snapshot = request
        .git_snapshot
        .clone()
        .ok_or_else(|| DomainError::conflict("request revision requires an uploaded snapshot"))?;
    Ok(RequestRevision {
        id: event.id.clone(),
        request_id: request.id.clone(),
        position: event.position,
        actor_user_id: event.actor_user_id.clone(),
        old_head_oid,
        new_head_oid,
        git_snapshot,
        created_at_unix: event.created_at_unix,
    })
}
