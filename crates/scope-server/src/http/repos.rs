use crate::{
    auth::{
        shoo::{
            ensure_user_for_identity, http_identity, identity_user_id, principal_for_repo,
            principal_for_user_id, require_identity,
        },
        tokens::{generate_first_push_token, generate_git_push_token},
    },
    error::ApiError,
    git::storage::delete_repo_storage,
    http::responses::*,
    persistence::{catalog_error, lock_catalog, persist_catalog, unix_now},
    state::AppState,
    state::{ensure_repo_read, find_repo, role_for_principal},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_git::{VirtualGitProjection, build_virtual_git_projection};
use scope_policy::{Principal, ScopePath, Visibility, VisibilityRule};
use scope_projection::{Projection, project_graph};
use scope_store::{RepoRole, RepoSettings};

pub(crate) async fn list_repos(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<RepoSummaryResponse>>, ApiError> {
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let catalog = lock_catalog(&state)?;
    let mut repositories = catalog
        .repositories_for_user(&user.id)
        .into_iter()
        .filter_map(|repo| repo_summary(&catalog, repo, &user.id))
        .collect::<Vec<_>>();
    repositories.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(Json(repositories))
}

pub(crate) async fn create_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateRepoRequest>,
) -> Result<Json<CreateRepoResponse>, ApiError> {
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let default_visibility = input.visibility.unwrap_or(Visibility::Private);

    let created = {
        let mut catalog = lock_catalog(&state)?;
        let mut staged = catalog.clone();
        let user = staged
            .users
            .get(&user.id)
            .cloned()
            .ok_or_else(|| ApiError::internal_message("signed-in user was not persisted"))?;
        let repo_id = staged
            .create_repository(&user, &input.name, default_visibility)
            .map_err(catalog_error)?
            .record
            .id
            .clone();
        let (secret, token) = generate_first_push_token(&user.id)?;
        let (push_secret, push_token) = generate_git_push_token(&user.id)?;
        let now = unix_now()?;
        {
            let repo = staged
                .repositories
                .get_mut(&repo_id)
                .expect("created repository must exist");
            repo.first_push_token = Some(token);
            repo.git_push_token = Some(push_token);
        }
        let repo = staged
            .repositories
            .get(&repo_id)
            .expect("created repository must exist");
        let summary = repo_summary(&staged, repo, &user.id).ok_or_else(|| {
            ApiError::internal_message("created repository is missing owner role")
        })?;
        let setup = repo_setup_response(
            &staged,
            repo,
            &user.id,
            now,
            Some(secret),
            Some(push_secret),
        )?;

        persist_catalog(&state, &staged)?;
        *catalog = staged;
        CreateRepoResponse {
            repo: summary,
            setup,
        }
    };

    Ok(Json(created))
}

pub(crate) async fn get_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoSummaryResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    let role = role_for_principal(&state, &repo, &principal)?;
    let staged_update_pending = role == Some(RepoRole::Owner) && repo.staged_update.is_some();
    let summary = RepoSummaryResponse {
        id: repo.record.id.clone(),
        owner_handle: repo.record.owner_handle.clone(),
        name: repo.record.name.clone(),
        lifecycle_state: repo.record.publication_state,
        default_visibility: repo.record.default_visibility,
        role,
        staged_update_pending,
    };

    Ok(Json(summary))
}

pub(crate) async fn delete_repo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<DeleteRepoResponse>, ApiError> {
    let identity = require_identity(&state, &headers).await?;
    let user = ensure_user_for_identity(&state, &identity)?;
    let repo_id = scope_store::repo_id(&owner, &repo_name);

    {
        let mut catalog = lock_catalog(&state)?;
        let mut staged = catalog.clone();
        let repo = staged
            .repositories
            .get(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        let principal = principal_for_user_id(repo, &user.id);
        if staged.role_for_principal(repo, &principal) != Some(RepoRole::Owner) {
            return Err(ApiError::not_found(format!(
                "repo {owner}/{repo_name} not found"
            )));
        }

        staged.repositories.remove(&repo_id);
        persist_catalog(&state, &staged)?;
        *catalog = staged;
    }

    delete_repo_storage(&state, &owner, &repo_name)?;

    Ok(Json(DeleteRepoResponse {
        id: repo_id,
        deleted: true,
    }))
}

pub(crate) async fn get_projection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<Projection>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    Ok(Json(project_graph(&repo.policy, &repo.graph, &principal)))
}

pub(crate) async fn get_git_projection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<VirtualGitProjection>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    let projection = project_graph(&repo.policy, &repo.graph, &principal);
    Ok(Json(build_virtual_git_projection(&projection)))
}

pub(crate) async fn get_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<Vec<RepoFileResponse>>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;

    Ok(Json(projected_files(&repo, &principal)))
}

pub(crate) async fn update_file_visibility(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<UpdateFileVisibilityRequest>,
) -> Result<Json<RepoFileResponse>, ApiError> {
    let identity = http_identity(&state, &headers).await?;
    let path = ScopePath::parse(&input.path).map_err(ApiError::bad_request)?;
    let repo_id = scope_store::repo_id(&owner, &repo_name);

    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    if role_for_principal(&state, &repo, &principal)? != Some(RepoRole::Owner) {
        return Err(ApiError::forbidden("owner role required"));
    }

    let owner_files = files_for_visibility_update(&repo, &principal)?;
    let selected_file = owner_files
        .iter()
        .find(|file| file.path == path.as_str())
        .ok_or_else(|| ApiError::not_found(format!("file {} not found", path.as_str())))?;
    if input.visibility == Visibility::Public && !selected_file.tracked {
        return Err(ApiError::bad_request(format!(
            "file {} must be tracked by Git before it can be made public",
            path.as_str()
        )));
    }

    let updated = {
        let mut catalog = lock_catalog(&state)?;
        let mut staged = catalog.clone();
        let repo = staged
            .repositories
            .get(&repo_id)
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        let user_id = identity
            .as_ref()
            .map(identity_user_id)
            .ok_or_else(|| ApiError::forbidden("owner role required"))?;
        let principal = principal_for_user_id(repo, &user_id);
        let role = staged.role_for_principal(repo, &principal);

        if role != Some(RepoRole::Owner) {
            return Err(ApiError::forbidden("owner role required"));
        }

        if input.visibility == Visibility::Public && !repo_has_file_for_review(repo, &path) {
            return Err(ApiError::bad_request(format!(
                "file {} must be tracked by Git before it can be made public",
                path.as_str()
            )));
        }

        {
            let repo = staged
                .repositories
                .get_mut(&repo_id)
                .expect("repo was already checked");
            let rule = match input.visibility {
                Visibility::Public => VisibilityRule::public(path.clone()),
                Visibility::Private => VisibilityRule::private(path.clone(), repo_owner_ids(repo)),
            };
            repo.policy.add_rule(rule).map_err(ApiError::bad_request)?;
        }

        persist_catalog(&state, &staged)?;
        let updated = staged
            .repositories
            .get(&repo_id)
            .expect("repo was already checked")
            .clone();
        *catalog = staged;
        updated
    };

    let principal = Principal {
        id: updated.record.owner_user_id.clone(),
        kind: scope_policy::PrincipalKind::User,
    };
    let updated_files = files_for_visibility_update(&updated, &principal)?;
    let file = updated_files
        .into_iter()
        .find(|file| file.path == path.as_str())
        .ok_or_else(|| ApiError::not_found(format!("file {} not found", path.as_str())))?;

    Ok(Json(file))
}

pub(crate) async fn get_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepoSettings>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;

    if role_for_principal(&state, &repo, &principal)? != Some(RepoRole::Owner) {
        return Err(ApiError::forbidden("owner role required"));
    }

    Ok(Json(repo.settings))
}

pub(crate) async fn update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<UpdateRepoSettingsRequest>,
) -> Result<Json<RepoSettings>, ApiError> {
    let identity = http_identity(&state, &headers).await?;
    let repo_id = scope_store::repo_id(&owner, &repo_name);
    let repo = find_repo(&state, &owner, &repo_name)?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;

    let mut catalog = lock_catalog(&state)?;
    let mut staged = catalog.clone();
    let repo = staged
        .repositories
        .get(&repo_id)
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    let user_id = identity
        .as_ref()
        .map(identity_user_id)
        .ok_or_else(|| ApiError::forbidden("owner role required"))?;
    let principal = principal_for_user_id(repo, &user_id);

    if staged.role_for_principal(repo, &principal) != Some(RepoRole::Owner) {
        return Err(ApiError::forbidden("owner role required"));
    }

    {
        let repo = staged
            .repositories
            .get_mut(&repo_id)
            .expect("repo was already checked");
        repo.settings.include_ignored_files = input.include_ignored_files;
        repo.settings.review_pushes_before_applying = input.review_pushes_before_applying;
    }

    persist_catalog(&state, &staged)?;
    let settings = staged
        .repositories
        .get(&repo_id)
        .expect("repo was already checked")
        .settings;
    *catalog = staged;

    Ok(Json(settings))
}
