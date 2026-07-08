use crate::domain::{
    policy::Visibility,
    repo_config::{
        ConfigVisibility, HistoryRewriteAction, HistoryRewriteRequest, RepoConfig,
        RepoConfigHistory, RepoConfigVisibility, RepoConfigVisibilityRule,
    },
    requests::{
        RequestActorRole, RequestBaseAudience, RequestDisposition, RequestEventKind, RequestState,
    },
    store::{
        FirstPushTokenStatus, RepoPublicationState, RepositoryActor, RepositoryInviteState,
        RepositoryMemberPermissions, StagedFileChangeKind,
    },
};
use crate::http::{responses::*, routes};
use std::{fs, path::PathBuf};
use ts_rs::TS;

#[test]
#[ignore = "exports TypeScript API types for the web contract check"]
fn export_api_types() {
    let ts_config = ts_rs::Config::new().with_large_int("number");
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output_path = std::env::var_os("SCOPE_API_TS_EXPORT_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("../web/src/api/types.generated.ts"));
    let declarations = [
        generated_header(),
        declaration::<Visibility>(&ts_config),
        declaration::<RepositoryActor>(&ts_config),
        declaration::<RepositoryMemberPermissions>(&ts_config),
        declaration::<RepositoryInviteState>(&ts_config),
        declaration::<RepoPublicationState>(&ts_config),
        declaration::<FirstPushTokenStatus>(&ts_config),
        declaration::<StagedFileChangeKind>(&ts_config),
        declaration::<ConfigVisibility>(&ts_config),
        declaration::<RepoConfig>(&ts_config),
        declaration::<RepoConfigVisibility>(&ts_config),
        declaration::<RepoConfigVisibilityRule>(&ts_config),
        declaration::<RepoConfigHistory>(&ts_config),
        declaration::<HistoryRewriteRequest>(&ts_config),
        declaration::<HistoryRewriteAction>(&ts_config),
        declaration::<RequestActorRole>(&ts_config),
        declaration::<RequestBaseAudience>(&ts_config),
        declaration::<RequestState>(&ts_config),
        declaration::<RequestDisposition>(&ts_config),
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
        declaration::<CompletePushIntentRequest>(&ts_config),
        declaration::<CompletePushIntentResponse>(&ts_config),
        declaration::<RepoInitResponse>(&ts_config),
        declaration::<RepoConfigResponse>(&ts_config),
        declaration::<FirstPushTokenResponse>(&ts_config),
        declaration::<GitPushTokenResponse>(&ts_config),
        declaration::<RepoFileResponse>(&ts_config),
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
        declaration::<RequestSummaryResponse>(&ts_config),
        declaration::<RequestPermissionsResponse>(&ts_config),
        declaration::<RequestMergeabilityStatus>(&ts_config),
        declaration::<RequestMergeabilityResponse>(&ts_config),
        declaration::<RequestSettlementResponse>(&ts_config),
        declaration::<RequestEventResponse>(&ts_config),
        declaration::<RequestReservationResponse>(&ts_config),
        declaration::<FinalizeRequestSubmissionRequest>(&ts_config),
        declaration::<CommentRequestRequest>(&ts_config),
        declaration::<NeedsResponseRequest>(&ts_config),
        declaration::<RespondRequestRequest>(&ts_config),
        declaration::<ResolveRequestRequest>(&ts_config),
        declaration::<MergeRequestRequest>(&ts_config),
        cli_auth_endpoint_declarations(),
    ]
    .join("\n\n");

    fs::write(output_path, format!("{declarations}\n")).expect("write generated API types");
}

fn declaration<T: TS>(config: &ts_rs::Config) -> String {
    format!("export {}", T::decl(config))
}

fn generated_header() -> String {
    [
        "// This file is generated from Rust API response/request types.",
        "// Run `cargo test --manifest-path api/Cargo.toml export_api_types -- --ignored` to update it.",
        "// Do not edit this file by hand.",
    ]
    .join("\n")
}

fn cli_auth_endpoint_declarations() -> String {
    let endpoints = [
        ("accountSession", routes::ACCOUNT_SESSION),
        ("cliSession", routes::CLI_SESSION),
        ("deviceLoginStart", routes::CLI_DEVICE_LOGIN),
        ("deviceLoginPoll", routes::CLI_DEVICE_LOGIN_POLL),
        ("browserLoginStart", routes::CLI_BROWSER_LOGIN),
        ("browserLoginExchange", routes::CLI_BROWSER_LOGIN_EXCHANGE),
        (
            "exchangeGrantExchange",
            routes::CLI_EXCHANGE_GRANTS_EXCHANGE,
        ),
    ];
    let body = endpoints
        .into_iter()
        .map(|(name, path)| format!("  {name}: \"{path}\","))
        .collect::<Vec<_>>()
        .join("\n");
    format!("export const CliAuthApiEndpoints = {{\n{body}\n}} as const;")
}
