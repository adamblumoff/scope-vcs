use crate::domain::{
    policy::Visibility,
    repo_config::{
        ConfigVisibility, HistoryRewriteAction, HistoryRewriteRequest, RepoConfig,
        RepoConfigHistory, RepoConfigVisibility, RepoConfigVisibilityRule,
    },
    requests::{
        RequestActorRole, RequestAudience, RequestDisposition, RequestEventKind, RequestState,
        ResolutionDisposition,
    },
    store::{
        FileChangeKind, FirstPushTokenStatus, RepoPublicationState, RepositoryActor,
        RepositoryInviteState, RepositoryMemberPermissions,
    },
};
use crate::http::{responses::*, routes};
use crate::repo_events::{RepoChangeEvent, RepoChangeKind};
use std::{fs, path::Path};
use ts_rs::TS;

pub(crate) fn export_api_types(output_path: &Path) {
    let ts_config = ts_rs::Config::new().with_large_int("number");
    let declarations = [
        generated_header(),
        declaration::<Visibility>(&ts_config),
        declaration::<RepositoryActor>(&ts_config),
        declaration::<RepositoryMemberPermissions>(&ts_config),
        declaration::<RepositoryInviteState>(&ts_config),
        declaration::<RepoPublicationState>(&ts_config),
        declaration::<RepoChangeEvent>(&ts_config),
        declaration::<FirstPushTokenStatus>(&ts_config),
        declaration::<FileChangeKind>(&ts_config),
        declaration::<ConfigVisibility>(&ts_config),
        declaration::<RepoConfig>(&ts_config),
        declaration::<RepoConfigVisibility>(&ts_config),
        declaration::<RepoConfigVisibilityRule>(&ts_config),
        declaration::<RepoConfigHistory>(&ts_config),
        declaration::<HistoryRewriteRequest>(&ts_config),
        declaration::<HistoryRewriteAction>(&ts_config),
        declaration::<RequestActorRole>(&ts_config),
        declaration::<RequestAudience>(&ts_config),
        declaration::<RequestState>(&ts_config),
        declaration::<RequestDisposition>(&ts_config),
        declaration::<ResolutionDisposition>(&ts_config),
        declaration::<GitOid>(&ts_config),
        declaration::<RequestEventKind>(&ts_config),
        declaration::<ProjectionPreviewAudience>(&ts_config),
        declaration::<ProjectionPreviewSource>(&ts_config),
        declaration::<AccountSessionResponse>(&ts_config),
        declaration::<UserResponse>(&ts_config),
        declaration::<SessionResponse>(&ts_config),
        declaration::<SessionIdentity>(&ts_config),
        declaration::<SessionRepo>(&ts_config),
        declaration::<SessionCapabilities>(&ts_config),
        declaration::<DeviceLoginStatus>(&ts_config),
        declaration::<DeviceLoginStartResponse>(&ts_config),
        declaration::<DeviceLoginPollResponse>(&ts_config),
        declaration::<DeviceLoginCompleteResponse>(&ts_config),
        declaration::<BrowserLoginStartRequest>(&ts_config),
        declaration::<BrowserLoginStartResponse>(&ts_config),
        declaration::<BrowserLoginCompleteResponse>(&ts_config),
        declaration::<BrowserLoginExchangeRequest>(&ts_config),
        declaration::<CliSessionTokenResponse>(&ts_config),
        declaration::<CliExchangeGrantResponse>(&ts_config),
        declaration::<CliExchangeGrantExchangeRequest>(&ts_config),
        declaration::<CliSessionsResponse>(&ts_config),
        declaration::<CliSessionResponse>(&ts_config),
        declaration::<RepoSummaryResponse>(&ts_config),
        declaration::<RepoRequestPermissionsResponse>(&ts_config),
        declaration::<CreateRepoRequest>(&ts_config),
        declaration::<CreateRepoResponse>(&ts_config),
        declaration::<DeleteRepoResponse>(&ts_config),
        declaration::<CreatePushIntentRequest>(&ts_config),
        declaration::<CreatePushIntentResponse>(&ts_config),
        declaration::<RepoInitResponse>(&ts_config),
        declaration::<RepoConfigResponse>(&ts_config),
        declaration::<FirstPushTokenResponse>(&ts_config),
        declaration::<GitPushTokenResponse>(&ts_config),
        declaration::<RepoFileResponse>(&ts_config),
        declaration::<RepoFileContentRequest>(&ts_config),
        declaration::<RepoFileContentResponse>(&ts_config),
        declaration::<RepositoryAccessResponse>(&ts_config),
        declaration::<RepositoryCollaborationResponse>(&ts_config),
        declaration::<RepositoryMemberResponse>(&ts_config),
        declaration::<RepositoryInviteResponse>(&ts_config),
        declaration::<CreateRepositoryInviteRequest>(&ts_config),
        declaration::<CreateRepositoryInviteResponse>(&ts_config),
        declaration::<UpdateRepositoryMemberRequest>(&ts_config),
        declaration::<RepositoryInviteLookupResponse>(&ts_config),
        declaration::<AcceptRepositoryInviteResponse>(&ts_config),
        declaration::<CommitHistoryRequest>(&ts_config),
        declaration::<CommitFileDiffRequest>(&ts_config),
        declaration::<RequestFileDiffRequest>(&ts_config),
        declaration::<RequestChangeBlockFilesResponse>(&ts_config),
        declaration::<ReviewFileContentResponse>(&ts_config),
        declaration::<ReviewFileDiffResponse>(&ts_config),
        declaration::<CommitHistoryResponse>(&ts_config),
        declaration::<CommitSummaryResponse>(&ts_config),
        declaration::<CommitDetailResponse>(&ts_config),
        declaration::<CommitFileResponse>(&ts_config),
        declaration::<ProjectionPreviewRequest>(&ts_config),
        declaration::<ProjectionPreviewResponse>(&ts_config),
        declaration::<ProjectionPreviewFileResponse>(&ts_config),
        declaration::<ProjectionPreviewCommitResponse>(&ts_config),
        declaration::<ProjectionPreviewCommitVisibilityResponse>(&ts_config),
        declaration::<ProjectionPreviewSummaryResponse>(&ts_config),
        declaration::<RequestListResponse>(&ts_config),
        declaration::<RequestDetailResponse>(&ts_config),
        declaration::<RequestMutationResponse>(&ts_config),
        declaration::<RequestListItemResponse>(&ts_config),
        declaration::<RequestSummaryResponse>(&ts_config),
        declaration::<RequestPermissionsResponse>(&ts_config),
        declaration::<RequestMergeabilityStatus>(&ts_config),
        declaration::<RequestMergeabilityResponse>(&ts_config),
        declaration::<RequestSettlementResponse>(&ts_config),
        declaration::<RequestSettlementPreviewResponse>(&ts_config),
        declaration::<RequestResolutionOptionResponse>(&ts_config),
        declaration::<RequestEventResponse>(&ts_config),
        declaration::<RequestEventPayload>(&ts_config),
        declaration::<RequestSettlement>(&ts_config),
        declaration::<RequestActorSummaryResponse>(&ts_config),
        declaration::<RequestDiscussionStatus>(&ts_config),
        declaration::<RequestDiscussionReplyResponse>(&ts_config),
        declaration::<RequestDiscussionSummaryResponse>(&ts_config),
        declaration::<RequestChangeBlockResponse>(&ts_config),
        declaration::<RequestDiscussionPageResponse>(&ts_config),
        declaration::<RequestDiscussionRepliesPageResponse>(&ts_config),
        declaration::<RequestDiscussionMutationResponse>(&ts_config),
        declaration::<RequestDiscussionReplyMutationResponse>(&ts_config),
        declaration::<RequestDiscussionChangesResponse>(&ts_config),
        declaration::<RequestDiscussionReadResponse>(&ts_config),
        declaration::<RequestActivityPageResponse>(&ts_config),
        declaration::<RequestDeleteResponse>(&ts_config),
        declaration::<StartRequestRequest>(&ts_config),
        declaration::<SubmitRequestRequest>(&ts_config),
        declaration::<UpdateRequestDescriptionRequest>(&ts_config),
        declaration::<CreateRequestDiscussionRequest>(&ts_config),
        declaration::<CreateRequestDiscussionReplyRequest>(&ts_config),
        declaration::<ReopenAndReplyRequest>(&ts_config),
        declaration::<MarkRequestDiscussionReadRequest>(&ts_config),
        declaration::<NeedsResponseRequest>(&ts_config),
        declaration::<RespondRequestRequest>(&ts_config),
        declaration::<ResolveRequestRequest>(&ts_config),
        declaration::<MergeRequestRequest>(&ts_config),
        declaration::<RepoChangeKind>(&ts_config),
        api_route_template_declarations(),
        api_path_builder_declaration(),
    ]
    .join("\n\n");

    fs::write(output_path, format!("{declarations}\n")).expect("write generated API types");
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
        ("repo", routes::REPO),
        ("repoConfig", routes::REPO_CONFIG),
        ("repoPushIntents", routes::REPO_PUSH_INTENTS),
        ("repoRequests", routes::REPO_REQUESTS),
        ("repoRequest", routes::REPO_REQUEST),
        ("repoSession", routes::REPO_SESSION),
        ("repoFiles", routes::REPO_FILES),
        ("repoFileContent", routes::REPO_FILE_CONTENT),
        (
            "repoRequestChangeBlockFiles",
            routes::REPO_REQUEST_CHANGE_BLOCK_FILES,
        ),
        (
            "repoRequestChangeBlockFileDiff",
            routes::REPO_REQUEST_CHANGE_BLOCK_FILE_DIFF,
        ),
        ("repoRequestDescription", routes::REPO_REQUEST_DESCRIPTION),
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
        (
            "repoRequestNeedsResponse",
            routes::REPO_REQUEST_NEEDS_RESPONSE,
        ),
        ("repoRequestRespond", routes::REPO_REQUEST_RESPOND),
        ("repoRequestResolve", routes::REPO_REQUEST_RESOLVE),
        ("repoRequestMerge", routes::REPO_REQUEST_MERGE),
        ("repoEvents", routes::REPO_EVENTS),
        ("repoCommits", routes::REPO_COMMITS),
        ("repoCommit", routes::REPO_COMMIT),
        ("repoCommitFileDiff", routes::REPO_COMMIT_FILE_DIFF),
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

fn generated_header() -> String {
    [
        "// This file is generated from Rust API response/request types.",
        "// Run `cargo run --manifest-path api/Cargo.toml --features type-export --bin export-api-types` to update it.",
        "// Do not edit this file by hand.",
    ]
    .join("\n")
}
