use scope_core::{
    auth::device::SessionIdentity,
    domain::{
        policy::Visibility,
        repo_config::RepoConfig,
        requests::{
            RequestActorRole, RequestBaseAudience, RequestDisposition, RequestEventKind,
            RequestMergeabilityStatus, RequestState, ResolutionDisposition,
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
    pub can_submit_request: bool,
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

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CompletePushIntentRequest {
    pub token: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestListResponse {
    pub requests: Vec<RequestSummaryResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestDetailResponse {
    pub request: RequestSummaryResponse,
    pub events: Vec<RequestEventResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestMutationResponse {
    pub request: RequestSummaryResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestDeleteResponse {
    pub deleted: bool,
    pub request: Option<RequestSummaryResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestSummaryResponse {
    pub id: String,
    pub title: String,
    pub author_user_id: String,
    pub editor_user_ids: Vec<String>,
    pub author_role: RequestActorRole,
    pub base_audience: RequestBaseAudience,
    pub target_branch: String,
    pub request_ref: String,
    pub base_main_oid: GitOid,
    pub head_oid: GitOid,
    pub state: RequestState,
    pub stake_credits: u32,
    pub disposition: Option<RequestDisposition>,
    pub settlement: Option<RequestSettlementResponse>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub resolved_at_unix: Option<u64>,
    pub permissions: RequestPermissionsResponse,
    pub mergeability: RequestMergeabilityResponse,
    pub resolution_options: Vec<RequestResolutionOptionResponse>,
    pub merge_settlement_preview: RequestSettlementPreviewResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestPermissionsResponse {
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
pub struct RequestSettlementResponse {
    pub disposition: RequestDisposition,
    pub stake_credits: u32,
    pub refunded_credits: u32,
    pub reward_credits: u32,
    pub burned_credits: u32,
    pub settled_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestSettlementPreviewResponse {
    pub stake_credits: u32,
    pub refunded_credits: u32,
    pub reward_credits: u32,
    pub burned_credits: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestResolutionOptionResponse {
    pub disposition: ResolutionDisposition,
    pub settlement: RequestSettlementPreviewResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestEventResponse {
    pub id: String,
    pub actor_user_id: String,
    pub kind: RequestEventKind,
    pub body: Option<String>,
    pub old_head_oid: Option<GitOid>,
    pub new_head_oid: Option<GitOid>,
    pub created_at_unix: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SubmitRequestRequest {
    pub head_oid: String,
    pub stake_credits: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StartRequestRequest {
    pub title: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CommentRequestRequest {
    pub body: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NeedsResponseRequest {
    pub body: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RespondRequestRequest {
    pub body: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResolveRequestRequest {
    pub disposition: ResolutionDisposition,
    pub body: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MergeRequestRequest {
    pub expected_main_oid: String,
    pub expected_head_oid: String,
    pub body: Option<String>,
}
