use super::{Request, RequestActorRole, RequestBaseAudience, RequestState};
use crate::domain::store::{RepositoryAccess, RepositoryActor};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestPermissions {
    pub can_comment: bool,
    pub can_pull_branch: bool,
    pub can_push_branch: bool,
    pub can_delete: bool,
    pub can_invite_editor: bool,
    pub can_mark_needs_response: bool,
    pub can_respond: bool,
    pub can_resolve: bool,
    pub can_merge: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestMergeabilityStatus {
    Ready,
    Closed,
    NotReady,
    NotMaintainer,
    MissingRequestBranch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestMergeability {
    pub status: RequestMergeabilityStatus,
    pub reason: Option<&'static str>,
}

pub fn request_actor_role(access: RepositoryAccess) -> RequestActorRole {
    match access.actor {
        RepositoryActor::Owner => RequestActorRole::Owner,
        RepositoryActor::Member => RequestActorRole::Member,
        RepositoryActor::Public => RequestActorRole::Public,
    }
}

pub fn request_base_audience(access: RepositoryAccess) -> RequestBaseAudience {
    match access.actor {
        RepositoryActor::Owner | RepositoryActor::Member => RequestBaseAudience::Private,
        RepositoryActor::Public => RequestBaseAudience::Public,
    }
}

pub fn request_visible_to_access(request: &Request, access: RepositoryAccess) -> bool {
    match access.actor {
        RepositoryActor::Owner | RepositoryActor::Member => true,
        RepositoryActor::Public => {
            request.author_role == RequestActorRole::Public
                && request.base_audience == request_base_audience(access)
        }
    }
}

pub fn request_permissions(
    request: &Request,
    access: RepositoryAccess,
    viewer_user_id: Option<&str>,
) -> RequestPermissions {
    let maintainer = matches!(
        access.actor,
        RepositoryActor::Owner | RepositoryActor::Member
    );
    let author = viewer_user_id == Some(request.author_user_id.as_str());
    let editor = viewer_user_id.is_some_and(|id| request.editor_user_ids.contains(id));
    let can_edit_branch = author || editor || maintainer;
    let open = !matches!(
        request.state,
        RequestState::Resolved | RequestState::Withdrawn
    );
    let submitted = request.state == RequestState::Submitted;
    let submitted_or_waiting = matches!(
        request.state,
        RequestState::Submitted | RequestState::NeedsResponse
    );
    RequestPermissions {
        can_comment: open && can_edit_branch,
        can_pull_branch: open && can_edit_branch && request.git_snapshot.is_some(),
        can_push_branch: open && can_edit_branch,
        can_delete: open && (author || maintainer),
        can_invite_editor: open && maintainer,
        can_mark_needs_response: submitted && maintainer,
        can_respond: open && author && request.state == RequestState::NeedsResponse,
        can_resolve: submitted_or_waiting && maintainer,
        can_merge: submitted && maintainer,
    }
}

pub fn request_mergeability(request: &Request, access: RepositoryAccess) -> RequestMergeability {
    let (status, reason) = if matches!(
        request.state,
        RequestState::Resolved | RequestState::Withdrawn
    ) {
        (RequestMergeabilityStatus::Closed, Some("request is closed"))
    } else if !matches!(
        access.actor,
        RepositoryActor::Owner | RepositoryActor::Member
    ) {
        (
            RequestMergeabilityStatus::NotMaintainer,
            Some("repo maintainer required"),
        )
    } else if request.state == RequestState::Working {
        (
            RequestMergeabilityStatus::NotReady,
            Some("request has not been submitted"),
        )
    } else if request.state == RequestState::NeedsResponse {
        (
            RequestMergeabilityStatus::NotReady,
            Some("request is waiting on contributor response"),
        )
    } else if request.git_snapshot.is_none() {
        (
            RequestMergeabilityStatus::MissingRequestBranch,
            Some("request branch has not been pushed"),
        )
    } else {
        (RequestMergeabilityStatus::Ready, None)
    };
    RequestMergeability { status, reason }
}
