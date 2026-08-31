use crate::{
    FileChangeKind, FirstPushTokenStatus, GitOid, RepoConfig, RepoLifecycleState, RepositoryActor,
    RequestActorRole, RequestAudience, RequestDiscussionStatus, RequestEventKind,
    RequestEventPayload, RequestMergeabilityStatus, RequestState, SessionIdentity, Visibility,
};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_request_payloads_preserve_optional_fields() {
        let submit: SubmitRequestRequest = serde_json::from_str("{}").expect("submit request");
        assert_eq!(serde_json::to_string(&submit).unwrap(), "{}");

        let edit: EditRequestIdentityRequest = serde_json::from_str(r#"{"title":"New title"}"#)
            .expect("identity edit with only a title");
        assert_eq!(edit.title.as_deref(), Some("New title"));
        assert_eq!(edit.description_markdown, None);
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct AccountSessionResponse {
    pub identity: Option<SessionIdentity>,
    pub user: Option<UserResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct UserResponse {
    pub id: String,
    pub handle: String,
    pub email: String,
    pub email_verified: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum DeviceLoginStatus {
    Pending,
    Complete,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct DeviceLoginStartResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_at_unix: u64,
    pub poll_interval_secs: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct DeviceLoginPollResponse {
    pub status: DeviceLoginStatus,
    pub session_token: Option<String>,
    pub expires_at_unix: u64,
    pub identity: Option<SessionIdentity>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct BrowserLoginStartRequest {
    pub callback_url: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct BrowserLoginStartResponse {
    pub request_id: String,
    pub request_secret: String,
    pub authorization_url: String,
    pub expires_at_unix: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct BrowserLoginExchangeRequest {
    pub request_secret: String,
    pub callback_code: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CliExchangeGrantExchangeRequest {
    pub exchange_token: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CliSessionTokenResponse {
    pub session_token: String,
    pub expires_at_unix: u64,
    pub identity: SessionIdentity,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CreateRepoRequest {
    pub name: String,
    pub file_default_visibility: Option<Visibility>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CreateRepoResponse {
    pub repo: RepoSummaryResponse,
    pub init: RepoInitResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoSummaryResponse {
    pub id: String,
    pub owner_handle: String,
    pub name: String,
    pub git_remote_url: String,
    pub lifecycle_state: RepoLifecycleState,
    pub change_version: u64,
    pub access: RepositoryAccessResponse,
    pub open_request_count: usize,
    pub request_permissions: RepoRequestPermissionsResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct OwnerProfileResponse {
    pub handle: String,
    pub repositories: Vec<RepoSummaryResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryAccessResponse {
    pub actor: RepositoryActor,
    pub can_read_private_files: bool,
    pub can_push: bool,
    pub can_change_file_visibility: bool,
    pub can_apply_changes: bool,
    pub can_manage_members: bool,
    pub can_delete_repo: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoRequestPermissionsResponse {
    pub can_start_request: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoInitResponse {
    pub repo: RepoSummaryResponse,
    pub git_remote_url: String,
    pub remote_name: String,
    pub push_branch: String,
    pub token: Option<FirstPushTokenResponse>,
    pub push_token: Option<GitPushTokenResponse>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct FirstPushTokenResponse {
    pub status: FirstPushTokenStatus,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
    pub used_at_unix: Option<u64>,
    pub secret: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct GitPushTokenResponse {
    pub created_at_unix: u64,
    pub secret: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoConfigResponse {
    pub config: RepoConfig,
    pub config_hash: String,
    pub lifecycle_state: RepoLifecycleState,
    pub access: RepositoryAccessResponse,
    pub head_oid: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CreatePushIntentRequest {
    pub head_oid: String,
    pub base_config_hash: String,
    pub config: RepoConfig,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CreatePushIntentResponse {
    pub token: String,
    pub base_head_oid: Option<GitOid>,
    pub expires_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestListResponse {
    pub requests: Vec<RequestListItemResponse>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDetailResponse {
    pub request: RequestSummaryResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CreateRequestRatingRequest {
    pub score: u8,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestRatingResponse {
    pub id: String,
    pub request_id: String,
    pub rater: RequestRatingParticipantResponse,
    pub subject: RequestRatingParticipantResponse,
    pub score: u8,
    pub reason: String,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestRatingParticipantResponse {
    pub id: String,
    pub handle: String,
    pub rating_score_sum: u64,
    pub rating_count: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestRatingsResponse {
    pub ratings: Vec<RequestRatingResponse>,
    pub eligible_subject: Option<RequestRatingParticipantResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestMutationResponse {
    pub request: RequestSummaryResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestCloseResponse {
    pub deleted: bool,
    pub request: Option<RequestSummaryResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestSummaryResponse {
    pub id: String,
    pub name: String,
    pub title: String,
    pub description_markdown: String,
    pub author_user_id: String,
    pub author_role: RequestActorRole,
    pub audience: RequestAudience,
    pub base_main_oid: GitOid,
    pub head_oid: GitOid,
    pub state: RequestState,
    pub activity_version: u64,
    pub submitted_at_unix: Option<u64>,
    pub closed_at_unix: Option<u64>,
    pub closed_by_user_id: Option<String>,
    pub merged_at_unix: Option<u64>,
    pub merged_by_user_id: Option<String>,
    pub merged_head_oid: Option<GitOid>,
    pub merged_main_oid: Option<GitOid>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub invitees: Vec<RequestInviteeResponse>,
    pub permissions: RequestPermissionsResponse,
    pub mergeability: RequestMergeabilityResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestInviteeResponse {
    pub user: RequestActorSummaryResponse,
    pub invited_by_user_id: String,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct AddRequestInviteeRequest {
    pub handle: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RemoveRequestInviteeRequest {
    pub handle: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestInviteeMutationResponse {
    pub request: RequestSummaryResponse,
    pub invitee: RequestInviteeResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct LeaveRequestResponse {
    pub invitee: RequestInviteeResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestListItemResponse {
    pub id: String,
    pub name: String,
    pub title: String,
    pub author_role: RequestActorRole,
    pub audience: RequestAudience,
    pub head_oid: GitOid,
    pub state: RequestState,
    pub submitted_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub mergeability: RequestMergeabilityResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestPermissionsResponse {
    pub can_view_activity: bool,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestMergeabilityResponse {
    pub status: RequestMergeabilityStatus,
    pub current_main_oid: Option<GitOid>,
    pub request_head_oid: GitOid,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestEventResponse {
    pub id: String,
    pub position: u64,
    pub actor: RequestActorSummaryResponse,
    pub kind: RequestEventKind,
    pub payload: RequestEventPayload,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestActorSummaryResponse {
    pub id: String,
    pub handle: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionReplyReferenceResponse {
    pub id: String,
    pub position: u64,
    pub author: RequestActorSummaryResponse,
    pub body_markdown: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionReplyResponse {
    pub id: String,
    pub discussion_id: String,
    pub position: u64,
    pub author: RequestActorSummaryResponse,
    pub body_markdown: String,
    pub reply_to: Option<RequestDiscussionReplyReferenceResponse>,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionSummaryResponse {
    pub id: String,
    pub request_id: String,
    pub client_discussion_id: String,
    pub opened_position: u64,
    pub last_activity_position: u64,
    pub author: RequestActorSummaryResponse,
    pub body_markdown: String,
    pub anchor: Option<RequestDiscussionAnchor>,
    pub status: RequestDiscussionStatus,
    pub reply_count: u64,
    pub read_through_position: u64,
    pub unread_count: u64,
    pub latest_replies: Vec<RequestDiscussionReplyResponse>,
    pub created_at_unix: u64,
    pub resolved_at_unix: Option<u64>,
    pub resolved_by: Option<RequestActorSummaryResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionAnchor {
    pub revision_id: String,
    pub revision_position: u64,
    pub commit_oid: Option<String>,
    pub path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionAnchorInput {
    pub revision_id: String,
    pub commit_oid: Option<String>,
    pub path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestRevisionCommitResponse {
    pub oid: String,
    pub parent_oids: Vec<String>,
    pub author: Option<String>,
    pub authored_at_unix: u64,
    pub message: String,
    pub change_count: usize,
    pub files: Vec<CommitFileResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CommitFileResponse {
    pub path: String,
    pub kind: FileChangeKind,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    pub old_oid: Option<String>,
    pub new_oid: Option<String>,
    pub visibility: Visibility,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RequestRevisionInspectionState {
    Complete,
    Incomplete,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestRevisionResponse {
    pub id: String,
    pub position: u64,
    pub actor: RequestActorSummaryResponse,
    pub old_head_oid: Option<String>,
    pub new_head_oid: Option<String>,
    pub commits: Vec<RequestRevisionCommitResponse>,
    pub inspection: RequestRevisionInspectionState,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestRevisionListResponse {
    pub review_revision_id: Option<String>,
    pub revisions: Vec<RequestRevisionResponse>,
    pub has_earlier_revisions: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionPageResponse {
    pub discussions: Vec<RequestDiscussionSummaryResponse>,
    pub next_cursor: Option<String>,
    pub snapshot_version: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionRepliesPageResponse {
    pub replies: Vec<RequestDiscussionReplyResponse>,
    pub next_before_position: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionMutationResponse {
    pub discussion: RequestDiscussionSummaryResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionReplyMutationResponse {
    pub discussion: RequestDiscussionSummaryResponse,
    pub reply: RequestDiscussionReplyResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionChangesResponse {
    pub discussions: Vec<RequestDiscussionSummaryResponse>,
    pub through_position: u64,
    pub has_more: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionReadResponse {
    pub read_through_position: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestActivityPageResponse {
    pub events: Vec<RequestEventResponse>,
    pub through_position: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct StartRequestRequest {
    pub name: String,
    pub title: Option<String>,
    pub audience: RequestAudience,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct SubmitRequestRequest {}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct EditRequestIdentityRequest {
    pub title: Option<String>,
    pub description_markdown: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CreateRequestDiscussionRequest {
    pub body_markdown: String,
    pub client_discussion_id: String,
    pub anchor: Option<RequestDiscussionAnchorInput>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CreateRequestDiscussionReplyRequest {
    pub body_markdown: String,
    pub client_reply_id: String,
    pub reply_to_reply_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ReopenAndReplyRequest {
    pub body_markdown: String,
    pub client_reply_id: String,
    pub reply_to_reply_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct MarkRequestDiscussionReadRequest {
    pub through_position: u64,
}
