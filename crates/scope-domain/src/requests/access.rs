use super::{Request, RequestActorRole, RequestAudience, RequestState};
use crate::store::{RepositoryAccess, RepositoryActor};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestViewer<'a> {
    pub access: RepositoryAccess,
    pub user_id: Option<&'a str>,
    pub is_invitee: bool,
}

impl<'a> RequestViewer<'a> {
    pub fn new(access: RepositoryAccess, user_id: Option<&'a str>, is_invitee: bool) -> Self {
        Self {
            access,
            user_id,
            is_invitee: user_id.is_some() && is_invitee,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestPermissions {
    pub can_open_discussion: bool,
    pub can_reply_to_discussion: bool,
    pub can_edit_identity: bool,
    pub can_pull_branch: bool,
    pub can_push_branch: bool,
    pub can_mark_ready: bool,
    pub can_return_to_working: bool,
    pub can_manage_invitees: bool,
    pub can_leave_request: bool,
    pub can_close: bool,
    pub can_merge: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestPolicyDecision {
    pub listable: bool,
    pub exact_visible: bool,
    pub discussion_visible: bool,
    pub activity_stream_visible: bool,
    pub git_advertised: bool,
    pub request_ref_readable: bool,
    pub branch_mutable: bool,
    pub counts_as_ready: bool,
    pub permissions: RequestPermissions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestMergeabilityStatus {
    Ready,
    Completed,
    Working,
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

pub fn request_policy(request: &Request, viewer: RequestViewer<'_>) -> RequestPolicyDecision {
    let maintainer = matches!(
        viewer.access.actor,
        RepositoryActor::Owner | RepositoryActor::Member
    );
    let authenticated = viewer.user_id.is_some();
    let author = viewer.user_id == Some(request.author_user_id.as_str());
    let invitee = viewer.is_invitee;
    let public = request.audience == RequestAudience::Public;
    let private = request.audience == RequestAudience::Private;
    let published = request.is_published();
    let working = request.state == RequestState::Working;
    let ready = request.state == RequestState::ReadyForReview;
    let completed = request.state == RequestState::Completed;

    let exact_visible = if private {
        maintainer
    } else if published {
        true
    } else {
        author || invitee
    };
    let listable = if private {
        maintainer
    } else if working {
        author || invitee
    } else {
        true
    };
    let git_advertised = if private {
        maintainer
    } else if working {
        author || invitee
    } else {
        true
    };
    let request_ref_readable = exact_visible;
    let branch_actor = if private {
        maintainer
    } else {
        author || invitee || maintainer
    };
    let branch_mutable = exact_visible && branch_actor && !completed;
    let discussion_visible = exact_visible;
    let activity_stream_visible = discussion_visible && (listable || published);
    // Public discussion stays open after completion; completed private requests are read-only.
    let can_discuss = discussion_visible && authenticated && (public || (maintainer && !completed));

    let permissions = RequestPermissions {
        can_open_discussion: can_discuss,
        can_reply_to_discussion: can_discuss,
        can_edit_identity: exact_visible && !completed && (author || maintainer),
        can_pull_branch: request_ref_readable,
        can_push_branch: branch_mutable,
        can_mark_ready: exact_visible && working && author,
        can_return_to_working: exact_visible && ready && author,
        can_manage_invitees: exact_visible && public && !completed && (author || maintainer),
        can_leave_request: exact_visible && public && invitee && !completed,
        can_close: exact_visible && working && author,
        can_merge: exact_visible && maintainer && request.merged_at_unix.is_none() && ready,
    };

    RequestPolicyDecision {
        listable,
        exact_visible,
        discussion_visible,
        activity_stream_visible,
        git_advertised,
        request_ref_readable,
        branch_mutable,
        counts_as_ready: ready && exact_visible,
        permissions,
    }
}

pub fn request_list_mergeability(
    state: RequestState,
    has_git_snapshot: bool,
    access: RepositoryAccess,
) -> RequestMergeability {
    let (status, reason) = if state == RequestState::Completed {
        (
            RequestMergeabilityStatus::Completed,
            Some("request is completed"),
        )
    } else if !matches!(
        access.actor,
        RepositoryActor::Owner | RepositoryActor::Member
    ) {
        (
            RequestMergeabilityStatus::NotMaintainer,
            Some("repo maintainer required"),
        )
    } else if state == RequestState::Working {
        (
            RequestMergeabilityStatus::Working,
            Some("request is not ready for review"),
        )
    } else if !has_git_snapshot {
        (
            RequestMergeabilityStatus::MissingRequestBranch,
            Some("request branch has not been pushed"),
        )
    } else {
        (RequestMergeabilityStatus::Ready, None)
    };
    RequestMergeability { status, reason }
}

pub fn request_mergeability(request: &Request, access: RepositoryAccess) -> RequestMergeability {
    request_list_mergeability(request.state, request.git_snapshot.is_some(), access)
}
