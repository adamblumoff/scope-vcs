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
    pub can_submit: bool,
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
    pub counts_as_open: bool,
    pub permissions: RequestPermissions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestMergeabilityStatus {
    Ready,
    Draft,
    Closed,
    Merged,
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
    let submitted = request.is_submitted();
    let terminal = request.is_terminal();
    let open = request.state() == RequestState::Open;

    let exact_visible = if private {
        maintainer
    } else if submitted {
        true
    } else {
        author || invitee
    };
    let listable = if private {
        maintainer
    } else if submitted {
        true
    } else {
        author || invitee
    };
    let git_advertised = if private {
        maintainer
    } else if submitted {
        true
    } else {
        author || invitee
    };
    let request_ref_readable = exact_visible;
    let branch_actor = if private {
        maintainer
    } else {
        author || invitee || maintainer
    };
    let branch_mutable = exact_visible && branch_actor && !terminal;
    let discussion_visible = exact_visible;
    let activity_stream_visible = discussion_visible && listable;
    let can_discuss = discussion_visible && authenticated && (public || (maintainer && !terminal));

    let permissions = RequestPermissions {
        can_open_discussion: can_discuss,
        can_reply_to_discussion: can_discuss,
        can_edit_identity: exact_visible && !terminal && (author || maintainer),
        can_pull_branch: request_ref_readable,
        can_push_branch: branch_mutable,
        can_submit: exact_visible && !submitted && author,
        can_manage_invitees: exact_visible && public && !terminal && (author || maintainer),
        can_leave_request: exact_visible && public && invitee && !terminal,
        can_close: exact_visible
            && ((request.state() == RequestState::Draft && author)
                || (open && (author || maintainer))),
        can_merge: exact_visible && maintainer && open,
    };

    RequestPolicyDecision {
        listable,
        exact_visible,
        discussion_visible,
        activity_stream_visible,
        git_advertised,
        request_ref_readable,
        branch_mutable,
        counts_as_open: open && exact_visible,
        permissions,
    }
}

pub fn request_list_mergeability(
    state: RequestState,
    has_git_snapshot: bool,
    access: RepositoryAccess,
) -> RequestMergeability {
    let (status, reason) = match state {
        RequestState::Closed => (RequestMergeabilityStatus::Closed, Some("request is closed")),
        RequestState::Merged => (RequestMergeabilityStatus::Merged, Some("request is merged")),
        RequestState::Draft => (
            RequestMergeabilityStatus::Draft,
            Some("request is not submitted"),
        ),
        RequestState::Open
            if !matches!(
                access.actor,
                RepositoryActor::Owner | RepositoryActor::Member
            ) =>
        {
            (
                RequestMergeabilityStatus::NotMaintainer,
                Some("repo maintainer required"),
            )
        }
        RequestState::Open if !has_git_snapshot => (
            RequestMergeabilityStatus::MissingRequestBranch,
            Some("request branch has not been pushed"),
        ),
        RequestState::Open => (RequestMergeabilityStatus::Ready, None),
    };
    RequestMergeability { status, reason }
}

pub fn request_mergeability(request: &Request, access: RepositoryAccess) -> RequestMergeability {
    request_list_mergeability(request.state(), request.git_snapshot.is_some(), access)
}
