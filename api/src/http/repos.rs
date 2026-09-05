use crate::{
    auth::{
        scope::{optional_scope_user, principal_for_scope_user, require_scope_user},
        tokens::{generate_first_push_token, generate_git_push_token},
    },
    error::ApiError,
    http::responses::*,
    http::{
        origins::public_git_origin,
        projection_preview::{ensure_projection_preview_access, projection_preview_repo},
    },
    persistence::unix_now,
    push_intents::repo_config_fingerprint,
    repo_access::find_repo,
    repo_events::RepoChangeReason,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use scope_api_contract::{
    CreatePushIntentRequest, CreatePushIntentResponse, CreateRepoRequest, CreateRepoResponse,
    OwnerProfileResponse, RepoConfigResponse, RepoSummaryResponse,
};
use scope_domain::repo_config::{
    is_repo_config_fingerprint, repo_config_fingerprint as domain_repo_config_fingerprint,
};
use scope_domain::{
    error::DomainError,
    policy::{ScopePath, Visibility},
};
use scope_domain::{
    landing_file::{MAX_REPOSITORY_LANDING_FILE_BYTES, REPOSITORY_LANDING_FILE_PATH},
    repo_actions::reviewed_update_domain_error,
};
use scope_domain::{
    repository::access::RepositoryActor,
    reviewed_updates::config::{ReviewedConfigUpdateInput, apply_reviewed_config_to_repo},
};
use scope_postgres::db::{RepoSummaryRead, RepositoryMutation};
use tracing::Instrument as _;

const MAX_PUSH_INTENT_CONFIG_BYTES: usize = 4096;

pub(crate) async fn get_owner_repositories(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(handle): Path<String>,
) -> Result<Json<OwnerProfileResponse>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;
    let profile = state
        .metadata
        .repositories()
        .owner_profile(&handle, user.as_ref().map(|user| user.id.as_str()))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("user {handle} not found")))?;
    let mut repositories = Vec::new();
    for summary in profile.repositories {
        repositories.push(repo_summary_response(&state, summary)?);
    }
    repositories.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(Json(OwnerProfileResponse {
        handle: profile.handle,
        repositories,
    }))
}

pub(crate) async fn create_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateRepoRequest>,
) -> Result<Json<CreateRepoResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let default_visibility = input
        .file_default_visibility
        .map(Into::into)
        .unwrap_or(Visibility::Private);
    let git_origin = public_git_origin(&state);
    let cleanup_state = state.clone();
    let (secret, token) = generate_first_push_token(&user.id)?;
    let (push_secret, push_token) = generate_git_push_token(&user.id)?;
    let now = unix_now()?;

    let repo = state
        .metadata
        .repositories()
        .create_repo_with_init_tokens(
            scope_postgres::db::CreateRepositoryCommand {
                owner_user_id: user.id.clone(),
                name: input.name.clone(),
                default_visibility,
                init_tokens: (token, push_token),
                now_unix: now,
            },
            &crate::persistence_ids::generate_persistence_id,
            move |cleanup| crate::git::storage::delete_repo_storage(&cleanup_state, cleanup),
        )
        .await?;

    let user_id = user.id.clone();
    let summary = repo_summary_for_user(&repo, &user_id, 0, git_origin)
        .ok_or_else(|| ApiError::internal_message("created repository is missing owner role"))?;
    let init = repo_init_response(
        &repo,
        &user_id,
        git_origin,
        now,
        Some(secret),
        Some(push_secret),
    )?;

    let created = CreateRepoResponse {
        repo: summary,
        init,
    };

    Ok(Json(created))
}

pub(crate) async fn get_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoSummaryResponse>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;
    let summary = state
        .metadata
        .repositories()
        .repo_summary(
            &owner,
            &repo_name,
            user.as_ref().map(|user| user.id.as_str()),
        )
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;

    Ok(Json(repo_summary_response(&state, summary)?))
}

pub(crate) async fn delete_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<DeleteRepoResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = find_repo(&state, &owner, &repo_name).await?;
    let incarnation = repo.incarnation();
    let delete_version = repo.record.change_version.saturating_add(1);
    let repo_id = state
        .metadata
        .repositories()
        .delete_repo(
            &owner,
            &repo_name,
            &incarnation,
            &user.id,
            unix_now()?,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await?;
    state
        .publish_repo_change(&incarnation, delete_version, RepoChangeReason::RepoDeleted)
        .await;

    crate::use_cases::content_cleanup::best_effort_drain_pending_repo_storage_deletions(&state)
        .await;
    Ok(Json(DeleteRepoResponse {
        id: repo_id,
        deleted: true,
    }))
}

pub(crate) async fn get_repo_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoConfigResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = state
        .metadata
        .repositories()
        .git_push_context(&owner, &repo_name, &user.id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    if repo.access.actor == RepositoryActor::Public {
        let full_repo = find_repo(&state, &owner, &repo_name).await?;
        let principal = principal_for_scope_user(&full_repo, Some(&user));
        crate::repo_access::ensure_repo_read(&state, &full_repo, &principal)?;
        return Err(ApiError::forbidden("repo membership required"));
    }

    Ok(Json(RepoConfigResponse {
        config_hash: repo_config_fingerprint(&repo.repo_config)?,
        lifecycle_state: repo.lifecycle_state.into(),
        access: repository_access_response(repo.access),
        head_oid: repo.git_head.as_ref().map(|head| head.head_oid.clone()),
        config: repo.repo_config.into(),
    }))
}

pub(crate) async fn create_push_intent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<CreatePushIntentRequest>,
) -> Result<Json<CreatePushIntentResponse>, ApiError> {
    let input_config: scope_domain::repo_config::RepoConfig = input.config.into();
    let user = require_scope_user(&state, &headers).await?;
    let repo = state
        .metadata
        .repositories()
        .git_push_context(&owner, &repo_name, &user.id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    let access = repo.access;

    if repo.lifecycle_state == scope_domain::repository::RepoLifecycleState::AwaitingFirstPush {
        if access.actor != RepositoryActor::Owner {
            return Err(ApiError::not_found(format!(
                "repo {owner}/{repo_name} not found"
            )));
        }
    } else if !access.can_push {
        return Err(ApiError::not_found(format!(
            "repo {owner}/{repo_name} not found"
        )));
    }

    let head_oid = git_oid_request("head_oid", &input.head_oid)?;
    validate_push_intent_config_transport(&input_config)?;
    let base_config_hash = repo_config_fingerprint(&repo.repo_config)?;
    if !is_repo_config_fingerprint(&input.base_config_hash) {
        return Err(ApiError::bad_request(
            "base_config_hash must be a SHA-256 hex digest",
        ));
    }
    if base_config_hash != input.base_config_hash && repo.repo_config != input_config {
        return Err(ApiError::conflict(
            "repo config changed since review; rerun scope review",
        ));
    }
    let base_head_oid = repo.git_head.as_ref().map(|head| head.head_oid.clone());
    let base_git_manifest_ref = repo
        .git_head
        .as_ref()
        .map(|head| head.manifest.content_ref.clone());
    let config_changed = repo.repo_config != input_config;
    if base_head_oid.as_deref() == Some(head_oid.as_str()) && config_changed {
        let author_id = user.id.clone();
        let config = input_config.clone();
        let expected_config_hash = base_config_hash.clone();
        let expected_manifest_ref = base_git_manifest_ref.clone();
        let changed = state
            .metadata
            .repositories()
            .mutate_repository(
                &owner,
                &repo_name,
                unix_now()?,
                &crate::persistence_ids::generate_persistence_id,
                move |repo| {
                    let access = repo.access_for_user_id(&author_id);
                    if !access.can_push {
                        return Err(DomainError::forbidden("push permission required"));
                    }
                    if !access.can_change_file_visibility && repo.repo_config != config {
                        return Err(DomainError::forbidden(
                            "file visibility permission required",
                        ));
                    }
                    if repo
                        .git_head
                        .as_ref()
                        .map(|head| &head.manifest.content_ref)
                        != expected_manifest_ref.as_ref()
                    {
                        return Err(DomainError::conflict(
                            "repo content changed since review; rerun scope push",
                        ));
                    }
                    if domain_repo_config_fingerprint(&repo.repo_config)
                        .map_err(DomainError::invariant_violation)?
                        != expected_config_hash
                    {
                        return Err(DomainError::conflict(
                            "repo config changed since review; rerun scope push",
                        ));
                    }
                    let changed = apply_reviewed_config_to_repo(
                        repo,
                        ReviewedConfigUpdateInput { author_id, config },
                    )
                    .map_err(reviewed_update_domain_error)?;
                    Ok(RepositoryMutation::new(changed))
                },
            )
            .await?;
        if changed {
            let repo = state
                .metadata
                .repositories()
                .git_push_context(&owner, &repo_name, &user.id)
                .await?
                .ok_or_else(|| {
                    ApiError::not_found(format!("repo {owner}/{repo_name} not found"))
                })?;
            state
                .publish_repo_change(
                    &repo.incarnation,
                    repo.change_version,
                    RepoChangeReason::ConfigApplied,
                )
                .await;
        }
    }
    let intent_base_config_hash =
        if config_changed && base_head_oid.as_deref() == Some(head_oid.as_str()) {
            repo_config_fingerprint(&input_config)?
        } else {
            base_config_hash
        };
    let intent = state.create_push_intent(
        &repo.repo_id,
        &user.id,
        &head_oid,
        input_config,
        intent_base_config_hash,
        base_git_manifest_ref,
    )?;

    Ok(Json(CreatePushIntentResponse {
        token: intent.token,
        base_head_oid: base_head_oid.map(git_oid_response).transpose()?,
        expires_at_unix: intent.expires_at_unix,
    }))
}

fn validate_push_intent_config_transport(
    config: &scope_domain::repo_config::RepoConfig,
) -> Result<(), ApiError> {
    config.validate().map_err(ApiError::bad_request)?;
    let bytes = serde_json::to_vec(config).map_err(ApiError::internal)?;
    if bytes.len() > MAX_PUSH_INTENT_CONFIG_BYTES {
        return Err(ApiError::bad_request(format!(
            "repo config exceeds {MAX_PUSH_INTENT_CONFIG_BYTES} bytes"
        )));
    }
    Ok(())
}

pub(crate) async fn get_projection_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(input): Query<ProjectionPreviewRequest>,
) -> Result<Json<ProjectionPreviewResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name).await?;
    let source = input.source.unwrap_or(ProjectionPreviewSource::Live);
    let user = optional_scope_user(&state, &headers).await?;
    let requester = principal_for_scope_user(&repo, user.as_ref());
    ensure_projection_preview_access(&state, &repo, &requester, input.audience, source)?;
    let include_private_counts =
        repo.access_for_principal(&requester).actor != RepositoryActor::Public;
    let preview_repo = projection_preview_repo(&repo, source)?;

    Ok(Json(projection_preview_response(
        &preview_repo,
        input.audience,
        source,
        include_private_counts,
    )?))
}

pub(crate) async fn get_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<Vec<RepoFileResponse>>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;
    let files = state
        .metadata
        .repositories()
        .repo_live_files(
            &owner,
            &repo_name,
            user.as_ref().map(|user| user.id.as_str()),
        )
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;

    Ok(Json(projection_file_responses(files)))
}

pub(crate) async fn get_file_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(input): Query<RepoFileContentRequest>,
) -> Result<Json<RepoFileContentResponse>, ApiError> {
    let path = ScopePath::parse(format!("/{}", input.path)).map_err(ApiError::bad_request)?;
    if path == ScopePath::root() {
        return Err(ApiError::bad_request("file path is required"));
    }
    let user = optional_scope_user(&state, &headers).await?;
    let projected = state
        .metadata
        .repositories()
        .repo_live_file_with_landing_content(
            &owner,
            &repo_name,
            user.as_ref().map(|user| user.id.as_str()),
            &path,
        )
        .await?
        .ok_or_else(|| ApiError::not_found("file not found"))?;
    let span = tracing::info_span!(
        "repo_file_content",
        owner = %owner,
        repo_name = %repo_name,
        file_path = %path.as_str(),
    );
    let content = if path.as_str() == REPOSITORY_LANDING_FILE_PATH {
        if projected.projected.blob.size_bytes > MAX_REPOSITORY_LANDING_FILE_BYTES as u64 {
            crate::http::file_diffs::binary_content_response(
                &projected.projected.blob.git_oid,
                projected.projected.blob.size_bytes,
            )
        } else {
            let landing_file = projected.landing_file.as_ref().ok_or_else(|| {
                tracing::error!(
                    owner,
                    repo_name,
                    file_path = REPOSITORY_LANDING_FILE_PATH,
                    oid = projected.projected.blob.git_oid,
                    "repository landing file snapshot is missing"
                );
                ApiError::internal_message("repository landing file snapshot requires rebuild")
            })?;
            if let Err(error) = landing_file.verify_source(&projected.projected.blob) {
                tracing::error!(
                    owner,
                    repo_name,
                    file_path = REPOSITORY_LANDING_FILE_PATH,
                    oid = projected.projected.blob.git_oid,
                    %error,
                    "repository landing file snapshot failed verification"
                );
                return Err(ApiError::from(error));
            }
            crate::http::file_diffs::review_content_response_for_bytes(
                &projected.projected.blob.git_oid,
                &landing_file.content_bytes,
            )
        }
    } else {
        let repo = find_repo(&state, &owner, &repo_name).await?;
        crate::http::file_diffs::review_content_response_for_blob(
            &state,
            &projected.projected.blob,
            repo.git_head
                .as_ref()
                .map(|head| (repo.incarnation(), head, repo.git_pack_spans.as_slice())),
        )
        .instrument(span)
        .await?
    };

    Ok(Json(RepoFileContentResponse {
        path: projected.projected.file.path.as_str().to_string(),
        oid: projected.projected.file.oid,
        visibility: projected.projected.file.visibility.into(),
        size_bytes: projected.projected.blob.size_bytes,
        content,
    }))
}

fn repo_summary_response(
    state: &AppState,
    summary: RepoSummaryRead,
) -> Result<RepoSummaryResponse, ApiError> {
    let request_permissions = repo_request_permissions_response(summary.access);
    let git_origin = public_git_origin(state);
    Ok(RepoSummaryResponse {
        id: summary.id,
        git_remote_url: repository_git_remote_url(
            git_origin,
            summary.access.actor,
            &summary.owner_handle,
            &summary.name,
        ),
        owner_handle: summary.owner_handle,
        name: summary.name,
        lifecycle_state: summary.lifecycle_state.into(),
        change_version: summary.change_version,
        access: repository_access_response(summary.access),
        open_request_count: summary.open_request_count,
        request_permissions,
    })
}
