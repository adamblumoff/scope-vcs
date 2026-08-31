use crate::{
    auth::scope::principal_for_user_id,
    error::ApiError,
    git::{
        cache::GitRepoHandle,
        projection_repo::projection_bare_repo_for_state,
        request_refs::{
            RequestRefUpdate, acquire_request_ref_update_lock, attach_visible_request_refs,
            create_request_receive_pack_staging_repo, install_request_receive_pack_hook,
            persist_request_ref_to_store, rollback_request_ref,
        },
        storage::remove_dir_if_exists,
    },
    persistence::unix_now,
    repo_access::find_repo,
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_domain::{
    projection::{ProjectionViewKey, project_graph},
    repository::{
        RepoLifecycleState, Repository, RepositoryIncarnation,
        access::{RepositoryAccess, RepositoryActor},
    },
    requests::{
        RecordRequestRevisionInput, Request, RequestAudience, RequestViewer, request_policy,
    },
};
use std::path::{Path, PathBuf};

pub(super) async fn actor_has_open_editable_request(
    state: &AppState,
    repo_id: &str,
    actor_user_id: &str,
    access: RepositoryAccess,
) -> Result<bool, ApiError> {
    for request in state
        .metadata
        .requests()
        .requests_by_repo_id(repo_id)
        .await?
    {
        let is_invitee = state
            .metadata
            .requests()
            .request_is_invitee(&request.id, actor_user_id)
            .await?;
        if request_actor_can_edit_ref(&request, actor_user_id, access, is_invitee) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) async fn prepare_request_staging_repo(
    state: &AppState,
    incarnation: &scope_domain::repository::RepositoryIncarnation,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
) -> Result<PathBuf, ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    if repo.incarnation() != *incarnation {
        return Err(ApiError::conflict(
            "repository was recreated during push preparation",
        ));
    }
    if repo.record.lifecycle_state != RepoLifecycleState::Ready {
        return Err(ApiError::not_found(format!(
            "repo {owner}/{repo_name} not found"
        )));
    }
    let access = repo.access_for_user_id(actor_user_id);
    if access.actor == RepositoryActor::Public
        && !actor_has_open_editable_request(state, &repo.record.id, actor_user_id, access).await?
    {
        return Err(ApiError::not_found(format!(
            "repo {owner}/{repo_name} not found"
        )));
    }

    let seed_repo = match access.actor {
        RepositoryActor::Public => public_projection_repo(state, &repo)?,
        RepositoryActor::Owner | RepositoryActor::Member => {
            if let Some(head) = repo.git_head.as_ref() {
                state.repository_engine.materialize_repository(
                    state,
                    &repo.incarnation(),
                    head,
                    &repo.git_pack_spans,
                )?
            } else {
                let principal = principal_for_user_id(&repo, actor_user_id);
                let projection = project_graph(
                    &repo.graph,
                    &repo.visibility_change_sets,
                    ProjectionViewKey::from_access(repo.access_for_principal(&principal)),
                );
                projection_bare_repo_for_state(
                    state,
                    &repo.incarnation(),
                    &projection,
                    repo.git_head.as_ref(),
                    &repo.git_pack_spans,
                )?
            }
        }
    };
    let staging_repo = create_request_receive_pack_staging_repo(state, incarnation, &seed_repo)?;
    if let Err(error) =
        seed_editable_request_refs_for_repo(state, &repo, actor_user_id, access, &staging_repo)
            .await
            .and_then(|()| install_request_receive_pack_hook(&staging_repo))
    {
        let _ = remove_dir_if_exists(&staging_repo);
        return Err(error);
    }
    Ok(staging_repo)
}

pub(super) async fn seed_editable_request_refs(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    staging_repo: &Path,
) -> Result<(), ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    let access = repo.access_for_user_id(actor_user_id);
    seed_editable_request_refs_for_repo(state, &repo, actor_user_id, access, staging_repo).await
}

async fn seed_editable_request_refs_for_repo(
    state: &AppState,
    repo: &Repository,
    actor_user_id: &str,
    access: RepositoryAccess,
    staging_repo: &Path,
) -> Result<(), ApiError> {
    let mut requests = Vec::new();
    for request in state
        .metadata
        .requests()
        .requests_by_repo_id(&repo.record.id)
        .await?
    {
        let is_invitee = state
            .metadata
            .requests()
            .request_is_invitee(&request.id, actor_user_id)
            .await?;
        let decision = request_policy(
            &request,
            RequestViewer::new(access, Some(actor_user_id), is_invitee),
        );
        if decision.branch_mutable && decision.git_advertised {
            requests.push(request);
        }
    }
    let public_base_repo = if access.actor != RepositoryActor::Public
        && requests.iter().any(|request| {
            request.audience == RequestAudience::Public && request.git_snapshot.is_none()
        }) {
        Some(public_projection_repo(state, repo)?)
    } else {
        None
    };
    attach_visible_request_refs(state, &requests, staging_repo, public_base_repo.as_deref())
}

fn public_projection_repo(state: &AppState, repo: &Repository) -> Result<GitRepoHandle, ApiError> {
    let projection = project_graph(
        &repo.graph,
        &repo.visibility_change_sets,
        ProjectionViewKey::Public,
    );
    projection_bare_repo_for_state(
        state,
        &repo.incarnation(),
        &projection,
        repo.git_head.as_ref(),
        &repo.git_pack_spans,
    )
}

pub(super) async fn persist_request_ref_revision(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    expected_incarnation: &RepositoryIncarnation,
    actor_user_id: &str,
    staging_repo: &Path,
    update: RequestRefUpdate,
) -> Result<(), ApiError> {
    let (repo, request) = ensure_request_ref_update_allowed(
        state,
        owner,
        repo_name,
        actor_user_id,
        &update.request_name,
    )
    .await?;
    let incarnation = repo.incarnation();
    if &incarnation != expected_incarnation {
        return Err(ApiError::conflict(
            "repository changed after receive-pack; retry the push",
        ));
    }
    let _lock = acquire_request_ref_update_lock(state, &incarnation, &update.request_ref)?;
    let request_audience = request.audience;
    let now_unix = unix_now()?;
    let expected_old_head_oid = update
        .old_head_oid
        .clone()
        .or_else(|| Some(request.head_oid.clone()));
    let persisted =
        persist_request_ref_to_store(state, &repo, staging_repo, &request, &update).await?;
    let mutation = state
        .metadata
        .requests()
        .record_request_revision(
            RecordRequestRevisionInput {
                request_id: request.id,
                actor_user_id: actor_user_id.to_string(),
                actor_can_edit: false,
                expected_old_head_oid,
                new_head_oid: update.new_head_oid.clone(),
                git_snapshot: persisted.git_snapshot.clone(),
                event_id: request_revision_event_id()?,
                body: None,
                now_unix,
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await;
    match mutation {
        Ok(_) => {
            state.product_analytics.capture(
                crate::product_analytics::ProductEvent::request_revised(
                    actor_user_id,
                    request_audience,
                ),
            );
            state
                .publish_request_summary_refresh(&incarnation, RepoChangeReason::RequestRevised)
                .await;
            persisted.fence.release().await;
        }
        Err(error) => {
            rollback_request_ref(
                state,
                &incarnation,
                &update.request_ref,
                persisted.previous_head,
            );
            crate::use_cases::content_cleanup::best_effort_cleanup_rollback_source_blobs(
                state,
                std::slice::from_ref(&persisted.git_snapshot),
            )
            .await;
            persisted.fence.release().await;
            return Err(error.into());
        }
    }
    Ok(())
}

async fn ensure_request_ref_update_allowed(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    request_name: &str,
) -> Result<(Repository, Request), ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    let access = repo.access_for_user_id(actor_user_id);
    let request = state
        .metadata
        .requests()
        .request_by_name(&repo.record.id, request_name)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    let is_invitee = state
        .metadata
        .requests()
        .request_is_invitee(&request.id, actor_user_id)
        .await?;
    if !request_actor_can_edit_ref(&request, actor_user_id, access, is_invitee) {
        return Err(ApiError::not_found("request not found"));
    }
    Ok((repo, request))
}

fn request_actor_can_edit_ref(
    request: &Request,
    actor_user_id: &str,
    access: RepositoryAccess,
    is_invitee: bool,
) -> bool {
    request_policy(
        request,
        RequestViewer::new(access, Some(actor_user_id), is_invitee),
    )
    .branch_mutable
}

fn request_revision_event_id() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to create request revision event id: {error}"
        ))
    })?;
    Ok(format!("event_request_revision_{}", hex::encode(bytes)))
}
