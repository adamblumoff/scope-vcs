use super::{entities, repository_from_model};
use crate::{
    domain::{
        requests::{Request, RequestActorRole, RequestBaseAudience, StartRequestInput},
        store::{RepoPublicationState, RepositoryActor, StoredRepository},
    },
    error::ApiError,
};
use sea_orm::EntityTrait;

pub(super) fn authorize_start_request(
    repo: &StoredRepository,
    mut input: StartRequestInput,
) -> Result<StartRequestInput, ApiError> {
    let (author_role, base_audience) = match repo.access_for_user_id(&input.author_user_id).actor {
        RepositoryActor::Owner => (RequestActorRole::Owner, RequestBaseAudience::Private),
        RepositoryActor::Member => (RequestActorRole::Member, RequestBaseAudience::Private),
        RepositoryActor::Public => {
            if repo.record.publication_state != RepoPublicationState::Published {
                return Err(ApiError::forbidden("published repository required"));
            }
            (RequestActorRole::Public, RequestBaseAudience::Public)
        }
    };
    input.author_role = author_role;
    input.base_audience = base_audience;
    Ok(input)
}

pub(super) fn ensure_request_maintainer(
    repo: &StoredRepository,
    user_id: &str,
) -> Result<(), ApiError> {
    match repo.access_for_user_id(user_id).actor {
        RepositoryActor::Owner | RepositoryActor::Member => Ok(()),
        RepositoryActor::Public => Err(ApiError::forbidden("repo maintainer required")),
    }
}

pub(super) fn request_actor_can_edit(
    repo: &StoredRepository,
    request: &Request,
    user_id: &str,
) -> bool {
    if request.author_user_id == user_id || request.editor_user_ids.contains(user_id) {
        return true;
    }
    matches!(
        repo.access_for_user_id(user_id).actor,
        RepositoryActor::Owner | RepositoryActor::Member
    )
}

pub(super) fn ensure_request_editor(
    repo: &StoredRepository,
    request: &Request,
    user_id: &str,
) -> Result<(), ApiError> {
    if request_actor_can_edit(repo, request, user_id) {
        Ok(())
    } else {
        Err(ApiError::forbidden("request edit access required"))
    }
}

pub(super) async fn ensure_user_exists<C>(conn: &C, user_id: &str) -> Result<(), ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    if entities::user::Entity::find_by_id(user_id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .is_some()
    {
        Ok(())
    } else {
        Err(ApiError::not_found("user not found"))
    }
}

pub(super) async fn repo_by_id<C>(conn: &C, repo_id: &str) -> Result<StoredRepository, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let repo = entities::repository::Entity::find_by_id(repo_id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("repo not found"))?;
    repository_from_model(conn, repo).await
}
