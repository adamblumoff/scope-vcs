use scope_core::{
    auth::device::SessionIdentity,
    domain::{
        policy::Visibility,
        repo_config::RepoConfig,
        requests::{
            RequestActorRole, RequestAssessmentOutcome, RequestAudience, RequestDiscussionStatus,
            RequestEventKind, RequestEventPayload, RequestMergeabilityStatus, RequestState,
        },
        store::{FirstPushTokenStatus, RepoPublicationState, RepositoryActor},
    },
};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{fmt, ops::Deref};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(type = "string"))]
pub struct GitOid(String);

impl GitOid {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GitOidParseError;

impl fmt::Display for GitOidParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Git OID must be exactly 40 hexadecimal characters")
    }
}

impl std::error::Error for GitOidParseError {}

impl TryFrom<&str> for GitOid {
    type Error = GitOidParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(GitOidParseError);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }
}

impl TryFrom<String> for GitOid {
    type Error = GitOidParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl<'de> Deserialize<'de> for GitOid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(de::Error::custom)
    }
}

impl From<GitOid> for String {
    fn from(value: GitOid) -> Self {
        value.0
    }
}

impl Deref for GitOid {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for GitOid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_oid_accepts_and_normalizes_canonical_sha1() {
        let oid = GitOid::try_from("ABCDEF0123456789ABCDEF0123456789ABCDEF01").unwrap();
        assert_eq!(oid.as_str(), "abcdef0123456789abcdef0123456789abcdef01");
        assert_eq!(
            serde_json::to_string(&oid).unwrap(),
            "\"abcdef0123456789abcdef0123456789abcdef01\""
        );
    }

    #[test]
    fn git_oid_rejects_non_sha1_values_at_construction_and_deserialization() {
        assert!(GitOid::try_from("head-1").is_err());
        assert!(GitOid::try_from(" aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_err());
        assert!(GitOid::try_from("abcdef0123456789abcdef0123456789abcdef0g").is_err());
        assert!(serde_json::from_str::<GitOid>("\"head-1\"").is_err());
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccountSessionResponse {
    pub identity: Option<SessionIdentity>,
    pub user: Option<UserResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UserResponse {
    pub id: String,
    pub handle: String,
    pub email: String,
    pub email_verified: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum DeviceLoginStatus {
    Pending,
    Complete,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceLoginStartResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_at_unix: u64,
    pub poll_interval_secs: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceLoginPollResponse {
    pub status: DeviceLoginStatus,
    pub session_token: Option<String>,
    pub expires_at_unix: u64,
    pub identity: Option<SessionIdentity>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BrowserLoginStartRequest {
    pub callback_url: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BrowserLoginStartResponse {
    pub request_id: String,
    pub request_secret: String,
    pub authorization_url: String,
    pub expires_at_unix: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BrowserLoginExchangeRequest {
    pub request_secret: String,
    pub callback_code: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CliExchangeGrantExchangeRequest {
    pub exchange_token: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CliSessionTokenResponse {
    pub session_token: String,
    pub expires_at_unix: u64,
    pub identity: SessionIdentity,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreateRepoRequest {
    pub name: String,
    pub visibility: Option<Visibility>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreateRepoResponse {
    pub repo: RepoSummaryResponse,
    pub init: RepoInitResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepoSummaryResponse {
    pub id: String,
    pub owner_handle: String,
    pub name: String,
    pub lifecycle_state: RepoPublicationState,
    pub default_visibility: Visibility,
    pub change_version: u64,
    pub access: RepositoryAccessResponse,
    pub open_request_count: usize,
    pub request_permissions: RepoRequestPermissionsResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepoRequestPermissionsResponse {
    pub can_start_request: bool,
    pub uses_credit_stake: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepoInitResponse {
    pub repo: RepoSummaryResponse,
    pub git_remote_url: String,
    pub remote_name: String,
    pub push_branch: String,
    pub token: Option<FirstPushTokenResponse>,
    pub push_token: Option<GitPushTokenResponse>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FirstPushTokenResponse {
    pub status: FirstPushTokenStatus,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
    pub used_at_unix: Option<u64>,
    pub secret: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GitPushTokenResponse {
    pub created_at_unix: u64,
    pub secret: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepoConfigResponse {
    pub config: RepoConfig,
    pub config_hash: String,
    pub lifecycle_state: RepoPublicationState,
    pub access: RepositoryAccessResponse,
    pub head_oid: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreatePushIntentRequest {
    pub head_oid: String,
    pub base_config_hash: String,
    pub config: RepoConfig,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreatePushIntentResponse {
    pub token: String,
    pub base_head_oid: Option<GitOid>,
    pub expires_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestListResponse {
    pub requests: Vec<RequestListItemResponse>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestDetailResponse {
    pub request: RequestSummaryResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestMutationResponse {
    pub request: RequestSummaryResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestCloseResponse {
    pub deleted: bool,
    pub request: Option<RequestSummaryResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    pub current_stake_credits: u32,
    pub first_ready_at_unix: Option<u64>,
    pub ready_at_unix: Option<u64>,
    pub held_at_unix: Option<u64>,
    pub held_by_user_id: Option<String>,
    pub assessment_outcome: Option<RequestAssessmentOutcome>,
    pub assessment_body_markdown: Option<String>,
    pub assessed_at_unix: Option<u64>,
    pub assessed_by_user_id: Option<String>,
    pub completed_at_unix: Option<u64>,
    pub completed_by_user_id: Option<String>,
    pub merged_at_unix: Option<u64>,
    pub merged_by_user_id: Option<String>,
    pub merged_head_oid: Option<GitOid>,
    pub merged_main_oid: Option<GitOid>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub permissions: RequestPermissionsResponse,
    pub mergeability: RequestMergeabilityResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestListItemResponse {
    pub id: String,
    pub name: String,
    pub title: String,
    pub author_role: RequestActorRole,
    pub audience: RequestAudience,
    pub head_oid: GitOid,
    pub state: RequestState,
    pub current_stake_credits: u32,
    pub assessment_outcome: Option<RequestAssessmentOutcome>,
    pub updated_at_unix: u64,
    pub mergeability: RequestMergeabilityResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestPermissionsResponse {
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestMergeabilityResponse {
    pub status: RequestMergeabilityStatus,
    pub current_main_oid: Option<GitOid>,
    pub request_head_oid: GitOid,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestEventResponse {
    pub id: String,
    pub position: u64,
    pub actor: RequestActorSummaryResponse,
    pub kind: RequestEventKind,
    pub payload: RequestEventPayload,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestActorSummaryResponse {
    pub id: String,
    pub handle: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestDiscussionReplyResponse {
    pub id: String,
    pub discussion_id: String,
    pub position: u64,
    pub author: RequestActorSummaryResponse,
    pub body_markdown: String,
    pub reply_to_reply_id: Option<String>,
    pub child_reply_count: u64,
    pub can_reply: bool,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestDiscussionSummaryResponse {
    pub id: String,
    pub request_id: String,
    pub client_discussion_id: String,
    pub opened_position: u64,
    pub last_activity_position: u64,
    pub author: RequestActorSummaryResponse,
    pub body_markdown: Option<String>,
    pub change_block: Option<RequestChangeBlockResponse>,
    pub status: RequestDiscussionStatus,
    pub reply_count: u64,
    pub unread_count: u64,
    pub latest_replies: Vec<RequestDiscussionReplyResponse>,
    pub created_at_unix: u64,
    pub resolved_at_unix: Option<u64>,
    pub resolved_by: Option<RequestActorSummaryResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestChangeBlockResponse {
    pub id: String,
    pub position: u64,
    pub old_head_oid: String,
    pub new_head_oid: String,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestDiscussionPageResponse {
    pub discussions: Vec<RequestDiscussionSummaryResponse>,
    pub next_cursor: Option<String>,
    pub snapshot_version: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestDiscussionRepliesPageResponse {
    pub replies: Vec<RequestDiscussionReplyResponse>,
    pub next_before_position: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestDiscussionMutationResponse {
    pub discussion: RequestDiscussionSummaryResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestDiscussionReplyMutationResponse {
    pub discussion: RequestDiscussionSummaryResponse,
    pub reply: RequestDiscussionReplyResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestDiscussionChangesResponse {
    pub discussions: Vec<RequestDiscussionSummaryResponse>,
    pub through_position: u64,
    pub has_more: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestDiscussionReadResponse {
    pub read_through_position: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestActivityPageResponse {
    pub events: Vec<RequestEventResponse>,
    pub through_position: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StartRequestRequest {
    pub name: String,
    pub title: Option<String>,
    pub audience: RequestAudience,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpdateRequestDescriptionRequest {
    pub description_markdown: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreateRequestDiscussionRequest {
    pub body_markdown: String,
    pub client_discussion_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreateRequestDiscussionReplyRequest {
    pub body_markdown: String,
    pub client_reply_id: String,
    pub reply_to_reply_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReopenAndReplyRequest {
    pub body_markdown: String,
    pub client_reply_id: String,
    pub reply_to_reply_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MarkRequestDiscussionReadRequest {
    pub through_position: u64,
}
