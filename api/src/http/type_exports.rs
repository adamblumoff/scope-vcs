use crate::http::{responses::*, routes};
use schemars::JsonSchema;
use scope_api_contract::{
    AccountSessionResponse, AddRequestInviteeRequest, BrowserLoginExchangeRequest,
    BrowserLoginStartRequest, BrowserLoginStartResponse, CliExchangeGrantExchangeRequest,
    CliSessionTokenResponse, ConfigVisibility, CreatePushIntentRequest, CreatePushIntentResponse,
    CreateRepoRequest, CreateRepoResponse, CreateRequestDiscussionReplyRequest,
    CreateRequestDiscussionRequest, CreateRequestRatingRequest, DeviceLoginPollResponse,
    DeviceLoginStartResponse, DeviceLoginStatus, EditRequestIdentityRequest, ErrorCode,
    ErrorFields, ErrorResponse, FileChangeKind, FirstPushTokenResponse, FirstPushTokenStatus,
    GitOid, GitPushTokenResponse, HistoryRewriteAction, HistoryRewriteRequest,
    LeaveRequestResponse, MarkRequestDiscussionReadRequest, OwnerProfileResponse,
    RemoveRequestInviteeRequest, ReopenAndReplyRequest, RepoChangeEvent, RepoChangeKind,
    RepoConfig, RepoConfigHistory, RepoConfigResponse, RepoConfigVisibility,
    RepoConfigVisibilityRule, RepoInitResponse, RepoLifecycleState, RepoRequestPermissionsResponse,
    RepoSummaryResponse, RepositoryAccessResponse, RepositoryActor, RepositoryInviteState,
    RepositoryMemberPermissions, RepositoryRunAttemptResponse, RepositoryRunAttemptState,
    RepositoryRunCacheColdReason, RepositoryRunCacheFinalState,
    RepositoryRunCacheObservationResponse, RepositoryRunCachePreparation,
    RepositoryRunCacheResponse, RepositoryRunCacheSetupObservationResponse,
    RepositoryRunDetailResponse, RepositoryRunJobDetailResponse, RepositoryRunJobResponse,
    RepositoryRunJobState, RepositoryRunState, RepositoryRunStepResponse, RepositoryRunStepState,
    RepositoryRunSummaryResponse, RepositoryRunTerminalReason, RepositoryRunTrigger,
    RequestActivityPageResponse, RequestActorRole, RequestActorSummaryResponse, RequestAudience,
    RequestCloseResponse, RequestDetailResponse, RequestDiscussionAnchor,
    RequestDiscussionAnchorInput, RequestDiscussionChangesResponse,
    RequestDiscussionMutationResponse, RequestDiscussionPageResponse,
    RequestDiscussionReadResponse, RequestDiscussionRepliesPageResponse,
    RequestDiscussionReplyMutationResponse, RequestDiscussionReplyReferenceResponse,
    RequestDiscussionReplyResponse, RequestDiscussionStatus, RequestDiscussionSummaryResponse,
    RequestEventKind, RequestEventPayload, RequestEventResponse, RequestIdentityAuditFact,
    RequestInviteeMutationResponse, RequestInviteeResponse, RequestListItemResponse,
    RequestListResponse, RequestMergeabilityResponse, RequestMergeabilityStatus,
    RequestMutationResponse, RequestPermissionsResponse, RequestQueueSection,
    RequestRatingParticipantResponse, RequestRatingResponse, RequestRatingsResponse,
    RequestRevisionCommitResponse, RequestRevisionInspectionState, RequestRevisionListResponse,
    RequestRevisionResponse, RequestState, RequestSummaryResponse, RunChangeKind, SessionIdentity,
    StartRequestRequest, SubmitRequestRequest, UserResponse, Visibility,
};
use std::{collections::BTreeMap, fs, path::Path};
use ts_rs::TS;

macro_rules! contract_declarations {
    ($config:expr; $($contract:ty),+ $(,)?) => {{
        vec![$(declaration::<$contract>($config)),+]
    }};
}

macro_rules! response_schemas {
    ($($response:ty),+ $(,)?) => {{
        let mut schemas = BTreeMap::new();
        $(
            let (name, response_schema) = schema::<$response>();
            assert!(
                schemas.insert(name.clone(), response_schema).is_none(),
                "duplicate API response schema {name}",
            );
        )+
        schemas
    }};
}

pub(crate) fn export_api_contract(output_path: &Path, schema_output_path: &Path) {
    let ts_config = ts_rs::Config::new().with_large_int("number");
    let type_declarations = contract_declarations!(
        &ts_config;
        ErrorCode,
        ErrorFields,
        ErrorResponse,
        Visibility,
        RepositoryActor,
        RepositoryMemberPermissions,
        RepositoryInviteState,
        RepoLifecycleState,
        RepoChangeEvent,
        FirstPushTokenStatus,
        FileChangeKind,
        ConfigVisibility,
        RepoConfig,
        RepoConfigVisibility,
        RepoConfigVisibilityRule,
        RepoConfigHistory,
        HistoryRewriteRequest,
        HistoryRewriteAction,
        RequestActorRole,
        RequestAudience,
        RequestState,
        GitOid,
        RequestEventKind,
        ProjectionPreviewAudience,
        ProjectionPreviewSource,
        AccountSessionResponse,
        UserResponse,
        SessionResponse,
        SessionIdentity,
        SessionRepo,
        SessionCapabilities,
        DeviceLoginStatus,
        DeviceLoginStartResponse,
        DeviceLoginPollResponse,
        DeviceLoginCompleteResponse,
        BrowserLoginStartRequest,
        BrowserLoginStartResponse,
        BrowserLoginCompleteResponse,
        BrowserLoginExchangeRequest,
        CliSessionTokenResponse,
        CliExchangeGrantResponse,
        CliExchangeGrantExchangeRequest,
        CliSessionsResponse,
        CliSessionResponse,
        RepoSummaryResponse,
        OwnerProfileResponse,
        RepoRequestPermissionsResponse,
        CreateRepoRequest,
        CreateRepoResponse,
        DeleteRepoResponse,
        CreatePushIntentRequest,
        CreatePushIntentResponse,
        RepoInitResponse,
        RepoConfigResponse,
        FirstPushTokenResponse,
        GitPushTokenResponse,
        RepoFileResponse,
        RepoFileContentRequest,
        RepoFileContentResponse,
        RepositoryAccessResponse,
        RepositoryCollaborationResponse,
        RepositoryMemberResponse,
        RepositoryInviteResponse,
        CreateRepositoryInviteRequest,
        CreateRepositoryInviteResponse,
        UpdateRepositoryMemberRequest,
        RepositoryInviteLookupResponse,
        AcceptRepositoryInviteResponse,
        HistoryPageRequest,
        HistoryEntryRequest,
        HistoryEntryFileDiffRequest,
        RequestFileDiffRequest,
        ReviewFileContentResponse,
        ReviewFileDiffResponse,
        HistoryPageResponse,
        HistoryEntrySummaryResponse,
        HistoryEntryKind,
        HistoryEntryDetailResponse,
        HistoryEntryFileResponse,
        HistoryVisibilitySummaryResponse,
        HistoryVisibilityChangeResponse,
        CommitFileResponse,
        ProjectionPreviewRequest,
        ProjectionPreviewResponse,
        ProjectionPreviewFileResponse,
        ProjectionPreviewCommitResponse,
        ProjectionPreviewCommitVisibilityResponse,
        ProjectionPreviewSummaryResponse,
        RequestQueueSection,
        RequestListResponse,
        RequestDetailResponse,
        CreateRequestRatingRequest,
        RequestRatingParticipantResponse,
        RequestRatingResponse,
        RequestRatingsResponse,
        RequestMutationResponse,
        RequestListItemResponse,
        RequestSummaryResponse,
        RequestInviteeResponse,
        AddRequestInviteeRequest,
        RemoveRequestInviteeRequest,
        RequestInviteeMutationResponse,
        LeaveRequestResponse,
        RequestPermissionsResponse,
        RequestMergeabilityStatus,
        RequestMergeabilityResponse,
        RequestEventResponse,
        RequestEventPayload,
        RequestIdentityAuditFact,
        RequestActorSummaryResponse,
        RequestDiscussionStatus,
        RequestDiscussionReplyReferenceResponse,
        RequestDiscussionReplyResponse,
        RequestDiscussionSummaryResponse,
        RequestDiscussionAnchor,
        RequestDiscussionAnchorInput,
        RequestRevisionCommitResponse,
        RequestRevisionInspectionState,
        RequestRevisionResponse,
        RequestRevisionListResponse,
        RequestDiscussionPageResponse,
        RequestDiscussionRepliesPageResponse,
        RequestDiscussionMutationResponse,
        RequestDiscussionReplyMutationResponse,
        RequestDiscussionChangesResponse,
        RequestDiscussionReadResponse,
        RequestActivityPageResponse,
        RequestCloseResponse,
        StartRequestRequest,
        SubmitRequestRequest,
        EditRequestIdentityRequest,
        CreateRequestDiscussionRequest,
        CreateRequestDiscussionReplyRequest,
        ReopenAndReplyRequest,
        MarkRequestDiscussionReadRequest,
        RepoChangeKind,
        RunChangeKind,
        RepositoryRunState,
        RepositoryRunTrigger,
        RepositoryRunSummaryResponse,
        RepositoryRunJobState,
        RepositoryRunJobResponse,
        RepositoryRunJobDetailResponse,
        RepositoryRunAttemptState,
        RepositoryRunStepState,
        RepositoryRunTerminalReason,
        RepositoryRunCacheColdReason,
        RepositoryRunCachePreparation,
        RepositoryRunCacheFinalState,
        RepositoryRunCacheObservationResponse,
        RepositoryRunCacheSetupObservationResponse,
        RepositoryRunCacheResponse,
        RepositoryRunStepResponse,
        RepositoryRunAttemptResponse,
        RepositoryRunWorkflowResponse,
        RepositoryRunWorkflowListResponse,
        RepositoryRunHistoryPageResponse,
        RepositoryRunLogResponse,
        RepositoryRunDetailResponse,
        RepositoryRunStepLogPageResponse,
    );
    let schemas = response_schemas!(
        AcceptRepositoryInviteResponse,
        AccountSessionResponse,
        BrowserLoginCompleteResponse,
        CliExchangeGrantResponse,
        CliSessionsResponse,
        CreateRepositoryInviteResponse,
        DeleteRepoResponse,
        DeviceLoginCompleteResponse,
        ErrorResponse,
        HistoryEntryDetailResponse,
        HistoryPageResponse,
        LeaveRequestResponse,
        OwnerProfileResponse,
        RepoChangeEvent,
        RepoFileContentResponse,
        RepoFileResponse,
        RepoSummaryResponse,
        RepositoryCollaborationResponse,
        RepositoryInviteLookupResponse,
        RepositoryInviteResponse,
        RepositoryMemberResponse,
        RepositoryRunDetailResponse,
        RepositoryRunHistoryPageResponse,
        RepositoryRunStepLogPageResponse,
        RepositoryRunWorkflowListResponse,
        RequestActivityPageResponse,
        RequestCloseResponse,
        RequestDetailResponse,
        RequestDiscussionChangesResponse,
        RequestDiscussionMutationResponse,
        RequestDiscussionPageResponse,
        RequestDiscussionReadResponse,
        RequestDiscussionRepliesPageResponse,
        RequestDiscussionReplyMutationResponse,
        RequestInviteeMutationResponse,
        RequestListResponse,
        RequestMutationResponse,
        RequestRatingResponse,
        RequestRatingsResponse,
        RequestRevisionListResponse,
        ReviewFileDiffResponse,
    );
    let declarations = [
        vec![generated_header()],
        type_declarations,
        vec![
            api_route_template_declarations(),
            api_path_builder_declaration(),
        ],
    ]
    .concat()
    .join("\n\n");

    fs::write(output_path, format!("{declarations}\n")).expect("write generated API types");
    let schema_document = serde_json::json!({
        "generated_from": "Rust API response/request types",
        "schemas": schemas,
    });
    fs::write(
        schema_output_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&schema_document)
                .expect("serialize generated API schemas"),
        ),
    )
    .expect("write generated API schemas");
}

fn api_route_template_declarations() -> String {
    let routes = [
        ("accountSession", routes::ACCOUNT_SESSION),
        ("cliDeviceLoginComplete", routes::CLI_DEVICE_LOGIN_COMPLETE),
        (
            "cliBrowserLoginComplete",
            routes::CLI_BROWSER_LOGIN_COMPLETE,
        ),
        ("cliExchangeGrants", routes::CLI_EXCHANGE_GRANTS),
        ("cliSessions", routes::CLI_SESSIONS),
        ("cliSessionById", routes::CLI_SESSION_BY_ID),
        ("repos", routes::REPOS),
        ("ownerRepositories", routes::OWNER_REPOSITORIES),
        ("repo", routes::REPO),
        ("repoConfig", routes::REPO_CONFIG),
        ("repoRunWorkflows", routes::REPO_RUN_WORKFLOWS),
        ("repoRuns", routes::REPO_RUNS),
        ("repoRunDetail", routes::REPO_RUN_DETAIL),
        ("repoRunStepLogs", routes::REPO_RUN_STEP_LOGS),
        ("repoRunCancel", routes::REPO_RUN_CANCEL),
        ("repoRunRetry", routes::REPO_RUN_RETRY),
        ("repoPushIntents", routes::REPO_PUSH_INTENTS),
        ("repoRequests", routes::REPO_REQUESTS),
        ("repoRequestQueue", routes::REPO_REQUEST_QUEUE),
        ("repoRequest", routes::REPO_REQUEST),
        ("repoRequestSubmit", routes::REPO_REQUEST_SUBMIT),
        ("repoRequestMerge", routes::REPO_REQUEST_MERGE),
        ("repoRequestRatings", routes::REPO_REQUEST_RATINGS),
        ("repoRequestInvitees", routes::REPO_REQUEST_INVITEES),
        ("repoRequestInviteesMe", routes::REPO_REQUEST_INVITEES_ME),
        ("repoSession", routes::REPO_SESSION),
        ("repoFiles", routes::REPO_FILES),
        ("repoFileContent", routes::REPO_FILE_CONTENT),
        ("repoRequestRevisions", routes::REPO_REQUEST_REVISIONS),
        (
            "repoRequestRevisionCommitFileDiff",
            routes::REPO_REQUEST_REVISION_COMMIT_FILE_DIFF,
        ),
        ("repoRequestDiscussions", routes::REPO_REQUEST_DISCUSSIONS),
        (
            "repoRequestDiscussionChanges",
            routes::REPO_REQUEST_DISCUSSION_CHANGES,
        ),
        (
            "repoRequestDiscussionReplies",
            routes::REPO_REQUEST_DISCUSSION_REPLIES,
        ),
        (
            "repoRequestDiscussionResolve",
            routes::REPO_REQUEST_DISCUSSION_RESOLVE,
        ),
        (
            "repoRequestDiscussionReopen",
            routes::REPO_REQUEST_DISCUSSION_REOPEN,
        ),
        (
            "repoRequestDiscussionReopenAndReply",
            routes::REPO_REQUEST_DISCUSSION_REOPEN_AND_REPLY,
        ),
        (
            "repoRequestDiscussionRead",
            routes::REPO_REQUEST_DISCUSSION_READ,
        ),
        ("repoRequestActivity", routes::REPO_REQUEST_ACTIVITY),
        ("repoEvents", routes::REPO_EVENTS),
        ("repoHistory", routes::REPO_HISTORY),
        ("repoHistoryEntry", routes::REPO_HISTORY_ENTRY),
        (
            "repoHistoryEntryFileDiff",
            routes::REPO_HISTORY_ENTRY_FILE_DIFF,
        ),
        ("repoMembers", routes::REPO_MEMBERS),
        ("repoInvites", routes::REPO_INVITES),
        ("repoInvite", routes::REPO_INVITE),
        ("repoMember", routes::REPO_MEMBER),
        ("repositoryInvite", routes::REPOSITORY_INVITE),
        ("repositoryInviteAccept", routes::REPOSITORY_INVITE_ACCEPT),
        ("repoProjectionPreview", routes::REPO_PROJECTION_PREVIEW),
        ("gitRepo", routes::GIT_REPO),
    ];
    let body = routes
        .into_iter()
        .map(|(name, path)| format!("  {name}: \"{path}\","))
        .collect::<Vec<_>>()
        .join("\n");
    format!("export const ApiRouteTemplates = {{\n{body}\n}} as const;")
}

fn api_path_builder_declaration() -> String {
    r#"export function buildApiPath(
  template: string,
  params: Readonly<Record<string, string>> = {},
): string {
  return template.replace(/\{([^}]+)\}/g, (_match, key: string) => {
    const value = params[key]
    if (value === undefined) throw new Error(`Missing API route parameter: ${key}`)
    return encodeURIComponent(value)
  })
}"#
    .to_string()
}

fn declaration<T: TS>(config: &ts_rs::Config) -> String {
    format!("export {}", T::decl(config))
}

fn schema<T: JsonSchema>() -> (String, serde_json::Value) {
    let name = T::schema_name().into_owned();
    let schema = schemars::generate::SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<T>();
    let mut schema = serde_json::to_value(schema).expect("serialize generated API schema");
    schema
        .as_object_mut()
        .expect("root API schema must be an object")
        .insert(
            "$id".to_string(),
            serde_json::Value::String(format!("scope://api/{name}")),
        );
    (name, schema)
}

fn generated_header() -> String {
    [
        "// This file is generated from Rust API response/request types.",
        "// Run `pnpm generate:api-contract` from web/ to update it.",
        "// Do not edit this file by hand.",
    ]
    .join("\n")
}
