use super::{Request, RequestActorRole, RequestAudience, RequestState};
use crate::domain::store::{RepositoryAccess, RepositoryActor};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestPermissions {
    pub can_open_discussion: bool,
    pub can_reply_to_discussion: bool,
    pub can_edit_description: bool,
    pub can_pull_branch: bool,
    pub can_push_branch: bool,
    pub can_mark_ready: bool,
    pub can_return_to_working: bool,
    pub can_manage_invitees: bool,
    pub can_hold: bool,
    pub can_assess: bool,
    pub can_close: bool,
    pub can_merge: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestMergeabilityStatus {
    Ready,
    Completed,
    Working,
    Held,
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

pub fn request_visible_audiences(access: RepositoryAccess) -> &'static [RequestAudience] {
    match access.actor {
        RepositoryActor::Owner | RepositoryActor::Member => {
            &[RequestAudience::Public, RequestAudience::Private]
        }
        RepositoryActor::Public => &[RequestAudience::Public],
    }
}

pub fn request_is_published(request: &Request) -> bool {
    request.is_published()
}

pub fn request_counts_as_open(request: &Request) -> bool {
    request.state != RequestState::Completed && request.is_published()
}

pub fn request_visible_to_access(request: &Request, access: RepositoryAccess) -> bool {
    request_visible_audiences(access).contains(&request.audience)
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
    let authenticated = viewer_user_id.is_some();
    let author = viewer_user_id == Some(request.author_user_id.as_str());
    let visible = request_visible_to_access(request, access);
    let can_collaborate = match request.audience {
        RequestAudience::Public => authenticated,
        RequestAudience::Private => maintainer,
    };
    let completed = request.state == RequestState::Completed;
    let held = request.held_at_unix.is_some();
    let working = request.state == RequestState::Working;
    let ready = request.state == RequestState::ReadyForReview;
    RequestPermissions {
        can_open_discussion: visible && !completed && can_collaborate,
        can_reply_to_discussion: visible && !completed && can_collaborate,
        can_edit_description: visible && working && !held && (author || maintainer),
        can_pull_branch: visible,
        can_push_branch: visible && !completed && !held && can_collaborate,
        can_mark_ready: visible && working && author,
        can_return_to_working: visible && ready && !held && author,
        can_manage_invitees: visible
            && request.audience == RequestAudience::Public
            && !completed
            && (maintainer || (author && !held)),
        can_hold: visible && ready && maintainer,
        can_assess: visible && ready && maintainer,
        can_close: visible && working && author,
        can_merge: visible
            && maintainer
            && request.merged_at_unix.is_none()
            && ((ready && !held)
                || (completed
                    && request.assessment_outcome
                        == Some(super::RequestAssessmentOutcome::Accepted))),
    }
}

pub fn request_list_mergeability(
    state: RequestState,
    assessment_outcome: Option<super::RequestAssessmentOutcome>,
    has_git_snapshot: bool,
    is_held: bool,
    is_merged: bool,
    access: RepositoryAccess,
) -> RequestMergeability {
    let completed_and_mergeable = state == RequestState::Completed
        && assessment_outcome == Some(super::RequestAssessmentOutcome::Accepted)
        && !is_merged;
    let (status, reason) = if state == RequestState::Completed && !completed_and_mergeable {
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
    } else if is_held {
        (RequestMergeabilityStatus::Held, Some("request is on hold"))
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
    request_list_mergeability(
        request.state,
        request.assessment_outcome,
        request.git_snapshot.is_some(),
        request.held_at_unix.is_some(),
        request.merged_at_unix.is_some(),
        access,
    )
}
