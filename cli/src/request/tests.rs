use super::{
    local::{
        RequestContext, maybe_request_branch_base_audience, parse_base_audience_config,
        store_request_metadata,
    },
    remote::RequestRemoteTarget,
    text::terminal_text,
};
use crate::{
    api::{
        RepoPublicationState, RepoRequestPermissionsResponse, RepoSummaryResponse,
        RepositoryAccessResponse, RepositoryActor, RequestMergeabilityResponse,
        RequestMergeabilityStatus, RequestPermissionsResponse, RequestSettlementPreviewResponse,
        RequestSummaryResponse, Visibility,
    },
    git_repo::GitRepo,
    test_support::TestDir,
};
use scope_core::domain::requests::{RequestActorRole, RequestBaseAudience, RequestState};

#[test]
fn terminal_text_replaces_control_characters() {
    assert_eq!(terminal_text("ok\u{1b}[31m\nnext\u{7}"), "ok [31m next ");
}

#[test]
fn base_audience_config_round_trips() {
    assert_eq!(
        parse_base_audience_config("public").unwrap(),
        RequestBaseAudience::Public
    );
    assert_eq!(
        parse_base_audience_config("private").unwrap(),
        RequestBaseAudience::Private
    );
    assert!(parse_base_audience_config("member").is_err());
}

#[test]
fn request_metadata_stores_request_base_audience_not_viewer_access() {
    let dir = TestDir::git_repo("request-audience", "request");
    let git_repo = GitRepo {
        root: dir.path.clone(),
    };
    let context = request_context_for_actor(RepositoryActor::Owner);
    let request = request_summary_with_audience(RequestBaseAudience::Public);

    store_request_metadata(&git_repo, "request", &context, &request).unwrap();

    assert_eq!(
        maybe_request_branch_base_audience(&git_repo).unwrap(),
        Some(RequestBaseAudience::Public)
    );
}

fn request_context_for_actor(actor: RepositoryActor) -> RequestContext {
    RequestContext {
        target: RequestRemoteTarget {
            remote: "origin".to_string(),
            access: crate::git_transport::GitAccess::Permissioned,
            public_url: "https://scope.example/owner/repo.git".to_string(),
            permissioned_url: "https://scope.example/owner/repo?git=permissioned".to_string(),
            owner: "owner".to_string(),
            repo: "repo".to_string(),
        },
        repo: RepoSummaryResponse {
            id: "owner/repo".to_string(),
            owner_handle: "owner".to_string(),
            name: "repo".to_string(),
            lifecycle_state: RepoPublicationState::Published,
            default_visibility: Visibility::Private,
            change_version: 1,
            access: RepositoryAccessResponse {
                actor,
                can_read_private_files: true,
                can_push: true,
                can_change_file_visibility: false,
                can_apply_changes: false,
                can_manage_members: false,
                can_delete_repo: false,
            },
            open_request_count: 1,
            request_permissions: RepoRequestPermissionsResponse {
                can_submit_request: true,
                uses_credit_stake: actor == RepositoryActor::Public,
            },
        },
    }
}

fn request_summary_with_audience(base_audience: RequestBaseAudience) -> RequestSummaryResponse {
    RequestSummaryResponse {
        id: "req_1".to_string(),
        title: "Request".to_string(),
        author_user_id: "public_user".to_string(),
        editor_user_ids: Vec::new(),
        author_role: RequestActorRole::Public,
        base_audience,
        target_branch: "main".to_string(),
        request_ref: "refs/scope/requests/req_1".to_string(),
        base_main_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .try_into()
            .unwrap(),
        head_oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .try_into()
            .unwrap(),
        state: RequestState::Working,
        stake_credits: 0,
        disposition: None,
        settlement: None,
        created_at_unix: 1,
        updated_at_unix: 1,
        resolved_at_unix: None,
        permissions: RequestPermissionsResponse {
            can_comment: true,
            can_pull_branch: true,
            can_push_branch: true,
            can_delete: true,
            can_invite_editor: false,
            can_mark_needs_response: false,
            can_respond: false,
            can_resolve: false,
            can_merge: false,
        },
        mergeability: RequestMergeabilityResponse {
            status: RequestMergeabilityStatus::NotReady,
            current_main_oid: None,
            request_head_oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .try_into()
                .unwrap(),
            reason: Some("request has not been submitted".to_string()),
        },
        resolution_options: Vec::new(),
        merge_settlement_preview: RequestSettlementPreviewResponse {
            stake_credits: 0,
            refunded_credits: 0,
            reward_credits: 0,
            burned_credits: 0,
        },
    }
}
