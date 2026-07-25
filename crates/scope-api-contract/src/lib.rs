//! Delivery contracts shared by the API and its Rust clients.
//!
//! Durable policy stays in `scope-core`; this crate owns only serialized shapes
//! and route construction.

mod types;

pub mod routes;
pub use scope_core::{
    auth::device::SessionIdentity,
    domain::{
        policy::Visibility,
        requests::{
            RequestActorRole, RequestAssessmentOutcome, RequestAudience, RequestDiscussionStatus,
            RequestEventKind, RequestEventPayload, RequestMergeabilityStatus, RequestQueueSection,
            RequestSettlement, RequestState,
        },
        store::{FirstPushTokenStatus, RepoPublicationState, RepositoryActor},
    },
};
pub use types::*;
