use scope_domain::{
    account::SessionIdentity as DomainSessionIdentity,
    policy::Visibility as DomainVisibility,
    requests::{
        RequestActorRole as DomainRequestActorRole, RequestAudience as DomainRequestAudience,
        RequestDiscussionStatus as DomainRequestDiscussionStatus,
        RequestEventKind as DomainRequestEventKind,
        RequestEventPayload as DomainRequestEventPayload,
        RequestIdentityAuditFact as DomainRequestIdentityAuditFact,
        RequestMergeabilityStatus as DomainRequestMergeabilityStatus,
        RequestQueueSection as DomainRequestQueueSection,
        RequestReviewExitReason as DomainRequestReviewExitReason,
        RequestState as DomainRequestState,
    },
    store::{
        FileChangeKind as DomainFileChangeKind, FirstPushTokenStatus as DomainFirstPushTokenStatus,
        RepoPublicationState as DomainRepoPublicationState,
        RepositoryActor as DomainRepositoryActor,
        RepositoryInviteState as DomainRepositoryInviteState,
        RepositoryMemberPermissions as DomainRepositoryMemberPermissions, UserAccount,
    },
};
use serde::{Deserialize, Serialize};

macro_rules! wire_enum {
    ($(#[$meta:meta])* $wire:ident => $domain:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        $(#[$meta])*
        pub enum $wire {
            $($variant),+
        }

        impl From<$domain> for $wire {
            fn from(value: $domain) -> Self {
                match value {
                    $($domain::$variant => Self::$variant),+
                }
            }
        }

        impl From<$wire> for $domain {
            fn from(value: $wire) -> Self {
                match value {
                    $($wire::$variant => Self::$variant),+
                }
            }
        }
    };
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionIdentity {
    pub user_id: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

impl From<DomainSessionIdentity> for SessionIdentity {
    fn from(value: DomainSessionIdentity) -> Self {
        Self {
            user_id: value.user_id,
            email: value.email,
            email_verified: value.email_verified,
        }
    }
}

impl From<SessionIdentity> for DomainSessionIdentity {
    fn from(value: SessionIdentity) -> Self {
        Self {
            user_id: value.user_id,
            email: value.email,
            email_verified: value.email_verified,
        }
    }
}

impl From<&UserAccount> for SessionIdentity {
    fn from(user: &UserAccount) -> Self {
        Self {
            user_id: user.id.clone(),
            email: (!user.email.is_empty()).then(|| user.email.clone()),
            email_verified: user.email_verified,
        }
    }
}

wire_enum!(Visibility => DomainVisibility { Public, Private });
wire_enum!(RepositoryActor => DomainRepositoryActor { Public, Member, Owner });
wire_enum!(RepositoryInviteState => DomainRepositoryInviteState {
    Pending,
    Accepted,
    Revoked,
    Expired,
});
wire_enum!(RepoPublicationState => DomainRepoPublicationState { Unpublished, Published });
wire_enum!(FirstPushTokenStatus => DomainFirstPushTokenStatus { Active, Expired, Used });
wire_enum!(FileChangeKind => DomainFileChangeKind { Added, Modified, Deleted });

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepositoryMemberPermissions {
    pub can_push: bool,
    pub can_change_file_visibility: bool,
    pub can_apply_changes: bool,
}

impl From<DomainRepositoryMemberPermissions> for RepositoryMemberPermissions {
    fn from(value: DomainRepositoryMemberPermissions) -> Self {
        Self {
            can_push: value.can_push,
            can_change_file_visibility: value.can_change_file_visibility,
            can_apply_changes: value.can_apply_changes,
        }
    }
}

impl From<RepositoryMemberPermissions> for DomainRepositoryMemberPermissions {
    fn from(value: RepositoryMemberPermissions) -> Self {
        Self {
            can_push: value.can_push,
            can_change_file_visibility: value.can_change_file_visibility,
            can_apply_changes: value.can_apply_changes,
        }
    }
}

wire_enum!(RequestActorRole => DomainRequestActorRole { Public, Member, Owner });
wire_enum!(RequestAudience => DomainRequestAudience { Public, Private });
wire_enum!(RequestState => DomainRequestState { Working, ReadyForReview, Completed });
wire_enum!(RequestReviewExitReason => DomainRequestReviewExitReason {
    AuthorReturned,
    RevisionPushed,
    ContentEdited,
});
wire_enum!(RequestEventKind => DomainRequestEventKind {
    Started,
    ReadyForReview,
    ReturnedToWorking,
    RevisionPushed,
    Merged,
    Closed,
    IdentityEdited,
    DiscussionResolved,
    DiscussionReopened,
});
wire_enum!(RequestMergeabilityStatus => DomainRequestMergeabilityStatus {
    Ready,
    Completed,
    Working,
    NotMaintainer,
    MissingRequestBranch,
});
wire_enum!(
    #[serde(rename_all = "snake_case")]
    #[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
    RequestQueueSection => DomainRequestQueueSection { YourWork, Ready, Completed }
);
wire_enum!(RequestDiscussionStatus => DomainRequestDiscussionStatus { Dormant, Open, Resolved });

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestIdentityAuditFact {
    pub title_sha256: String,
    pub title_byte_count: u64,
    pub description_sha256: String,
    pub description_byte_count: u64,
}

impl From<DomainRequestIdentityAuditFact> for RequestIdentityAuditFact {
    fn from(value: DomainRequestIdentityAuditFact) -> Self {
        Self {
            title_sha256: value.title_sha256,
            title_byte_count: value.title_byte_count,
            description_sha256: value.description_sha256,
            description_byte_count: value.description_byte_count,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum RequestEventPayload {
    Started {
        title: String,
        description_markdown: String,
    },
    ReadyForReview {
        head_oid: String,
    },
    ReturnedToWorking {
        head_oid: String,
        reason: RequestReviewExitReason,
    },
    RevisionPushed {
        old_head_oid: String,
        new_head_oid: String,
        note: Option<String>,
    },
    Merged {
        head_oid: String,
        main_oid: String,
    },
    Closed {
        head_oid: String,
    },
    IdentityEdited {
        before: RequestIdentityAuditFact,
        after: RequestIdentityAuditFact,
    },
    DiscussionResolved {
        discussion_id: String,
    },
    DiscussionReopened {
        discussion_id: String,
    },
}

impl From<DomainRequestEventPayload> for RequestEventPayload {
    fn from(value: DomainRequestEventPayload) -> Self {
        match value {
            DomainRequestEventPayload::Started {
                title,
                description_markdown,
            } => Self::Started {
                title,
                description_markdown,
            },
            DomainRequestEventPayload::ReadyForReview { head_oid } => {
                Self::ReadyForReview { head_oid }
            }
            DomainRequestEventPayload::ReturnedToWorking { head_oid, reason } => {
                Self::ReturnedToWorking {
                    head_oid,
                    reason: reason.into(),
                }
            }
            DomainRequestEventPayload::RevisionPushed {
                old_head_oid,
                new_head_oid,
                note,
            } => Self::RevisionPushed {
                old_head_oid,
                new_head_oid,
                note,
            },
            DomainRequestEventPayload::Merged { head_oid, main_oid } => {
                Self::Merged { head_oid, main_oid }
            }
            DomainRequestEventPayload::Closed { head_oid } => Self::Closed { head_oid },
            DomainRequestEventPayload::IdentityEdited { before, after } => Self::IdentityEdited {
                before: before.into(),
                after: after.into(),
            },
            DomainRequestEventPayload::DiscussionResolved { discussion_id } => {
                Self::DiscussionResolved { discussion_id }
            }
            DomainRequestEventPayload::DiscussionReopened { discussion_id } => {
                Self::DiscussionReopened { discussion_id }
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepoChangeEvent {
    pub repo_id: String,
    pub version: u64,
    pub kind: RepoChangeKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum RepoChangeKind {
    Connected,
    Lagged,
    RepositoryChanged {
        reason: String,
    },
    RequestTimelineChanged {
        request_id: String,
        discussion_id: String,
        through_position: u64,
        audience: RequestAudience,
    },
}
