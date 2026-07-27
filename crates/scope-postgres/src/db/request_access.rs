use super::{acquire_aggregate_lock, entities, repository_from_model, request_rows::request_by_id};
use sea_orm::EntityTrait;
use {
    crate::error::PostgresError,
    scope_domain::{
        requests::{
            Request, RequestActorRole, RequestAudience, RequestPolicyDecision, RequestViewer,
            StartRequestInput, request_policy,
        },
        store::{RepoPublicationState, RepositoryActor, StoredRepository},
    },
};

pub(super) fn authorize_start_request(
    repo: &StoredRepository,
    mut input: StartRequestInput,
) -> Result<StartRequestInput, PostgresError> {
    let author_role = match repo.access_for_user_id(&input.author_user_id).actor {
        RepositoryActor::Owner => RequestActorRole::Owner,
        RepositoryActor::Member => RequestActorRole::Member,
        RepositoryActor::Public => {
            if repo.record.publication_state != RepoPublicationState::Published {
                return Err(PostgresError::permission_denied(
                    "published repository required",
                ));
            }
            input.audience = RequestAudience::Public;
            RequestActorRole::Public
        }
    };
    input.author_role = author_role;
    Ok(input)
}

pub(super) async fn request_policy_for_user<C>(
    conn: &C,
    repo: &StoredRepository,
    request: &Request,
    user_id: &str,
) -> Result<RequestPolicyDecision, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let is_invitee =
        super::request_invitees::request_is_invitee(conn, &request.id, user_id).await?;
    Ok(request_policy(
        request,
        RequestViewer::new(repo.access_for_user_id(user_id), Some(user_id), is_invitee),
    ))
}

pub(super) async fn ensure_user_exists<C>(conn: &C, user_id: &str) -> Result<(), PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    if entities::user::Entity::find_by_id(user_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .is_some()
    {
        Ok(())
    } else {
        Err(PostgresError::not_found("user not found"))
    }
}

pub(super) async fn repo_by_id<C>(
    conn: &C,
    repo_id: &str,
) -> Result<StoredRepository, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let repo = entities::repository::Entity::find_by_id(repo_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("repo not found"))?;
    repository_from_model(conn, repo).await
}

pub(super) async fn lock_request_repository<C>(
    conn: &C,
    request_id: &str,
) -> Result<(StoredRepository, Request), PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let observed = request_by_id(conn, request_id)
        .await?
        .ok_or_else(|| PostgresError::not_found("request not found"))?;
    acquire_aggregate_lock(conn, "repository", &observed.repo_id).await?;
    acquire_aggregate_lock(conn, "request", request_id).await?;
    let request = request_by_id(conn, request_id)
        .await?
        .filter(|request| request.repo_id == observed.repo_id)
        .ok_or_else(|| PostgresError::not_found("request not found"))?;
    let repo = repo_by_id(conn, &request.repo_id).await?;
    Ok((repo, request))
}
