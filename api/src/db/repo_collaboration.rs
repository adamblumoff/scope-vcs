use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, entities,
    repo_effects::save_repo_mutation, repository_from_model, run_api_db_on,
};
use crate::domain::{
    repo_collaboration::{
        CreateRepositoryInviteCommand, accept_repository_invite,
        create_or_refresh_repository_invite, remove_repository_member,
        update_repository_member_permissions,
    },
    store::{
        RepositoryInvite, RepositoryMember, RepositoryMemberPermissions, UserAccount,
        normalize_repository_invite_email, repo_id,
    },
};
use crate::error::ApiError;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, TransactionTrait};
use std::sync::Arc;

impl MetadataStore {
    pub(crate) fn create_repository_invite(
        &self,
        owner: &str,
        name: &str,
        owner_user: UserAccount,
        invited_email: String,
        permissions: RepositoryMemberPermissions,
        invite_id: String,
        token_hash: String,
        now_unix: u64,
    ) -> Result<RepositoryInvite, ApiError> {
        let repo_id = repo_id(owner, name);
        let owner_name = owner.to_string();
        let name = name.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let row = entities::repository::Entity::find_by_id(repo_id)
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| {
                            ApiError::not_found(format!("repo {owner_name}/{name} not found"))
                        })?;
                    let mut repo = repository_from_model(&tx, row).await?;
                    let invitee = user_by_normalized_email(&tx, &invited_email).await?;
                    let mutation = create_or_refresh_repository_invite(
                        &mut repo,
                        CreateRepositoryInviteCommand {
                            id: invite_id,
                            owner: &owner_user,
                            invited_email,
                            invitee: invitee.as_ref(),
                            permissions,
                            token_hash,
                            now_unix,
                        },
                    )?;
                    save_repo_mutation(&tx, &repo, &mutation_effects_none()).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(mutation)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let invitee = catalog.users.values().find(|user| {
                    normalize_repository_invite_email(&user.email)
                        == normalize_repository_invite_email(&invited_email)
                });
                let repo = catalog.repositories.get_mut(&repo_id).ok_or_else(|| {
                    ApiError::not_found(format!("repo {owner_name}/{name} not found"))
                })?;
                let invite = create_or_refresh_repository_invite(
                    repo,
                    CreateRepositoryInviteCommand {
                        id: invite_id,
                        owner: &owner_user,
                        invited_email,
                        invitee,
                        permissions,
                        token_hash,
                        now_unix,
                    },
                )?;
                Ok(invite)
            }),
        }
    }

    pub(crate) fn update_repository_member_permissions(
        &self,
        owner: &str,
        name: &str,
        owner_user_id: &str,
        member_user_id: &str,
        permissions: RepositoryMemberPermissions,
        now_unix: u64,
    ) -> Result<RepositoryMember, ApiError> {
        let owner_user_id = owner_user_id.to_string();
        let member_user_id = member_user_id.to_string();
        mutate_repository_member(self, owner, name, move |repo| {
            update_repository_member_permissions(
                repo,
                &owner_user_id,
                &member_user_id,
                permissions,
                now_unix,
            )
        })
    }

    pub(crate) fn remove_repository_member(
        &self,
        owner: &str,
        name: &str,
        owner_user_id: &str,
        member_user_id: &str,
    ) -> Result<RepositoryMember, ApiError> {
        let owner_user_id = owner_user_id.to_string();
        let member_user_id = member_user_id.to_string();
        mutate_repository_member(self, owner, name, move |repo| {
            remove_repository_member(repo, &owner_user_id, &member_user_id)
        })
    }

    pub(crate) fn repository_invite_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<(crate::domain::store::StoredRepository, RepositoryInvite), ApiError> {
        let token_hash = token_hash.to_string();
        self.read(move |catalog| {
            for repo in catalog.repositories.values() {
                if let Some(invite) = repo
                    .invitations
                    .iter()
                    .find(|invite| invite.token_hash == token_hash)
                {
                    return Ok((repo.clone(), invite.clone()));
                }
            }
            Err(ApiError::not_found("repository invite not found"))
        })
    }

    pub(crate) fn accept_repository_invite(
        &self,
        token_hash: &str,
        user: UserAccount,
        now_unix: u64,
    ) -> Result<(crate::domain::store::StoredRepository, RepositoryMember), ApiError> {
        let token_hash = token_hash.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let invite = entities::repository_invite::Entity::find()
                        .filter(
                            entities::repository_invite::Column::TokenHash.eq(token_hash.clone()),
                        )
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| ApiError::not_found("repository invite not found"))?;
                    let row = entities::repository::Entity::find_by_id(invite.repo_id)
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| ApiError::not_found("repository invite not found"))?;
                    let mut repo = repository_from_model(&tx, row).await?;
                    let member = accept_repository_invite(&mut repo, &user, &token_hash, now_unix)?;
                    save_repo_mutation(&tx, &repo, &mutation_effects_none()).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok((repo, member))
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo_id = catalog
                    .repositories
                    .values()
                    .find(|repo| {
                        repo.invitations
                            .iter()
                            .any(|invite| invite.token_hash == token_hash)
                    })
                    .map(|repo| repo.record.id.clone())
                    .ok_or_else(|| ApiError::not_found("repository invite not found"))?;
                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .ok_or_else(|| ApiError::not_found("repository invite not found"))?;
                let member = accept_repository_invite(repo, &user, &token_hash, now_unix)?;
                Ok((repo.clone(), member))
            }),
        }
    }
}

fn mutate_repository_member<F>(
    store: &MetadataStore,
    owner: &str,
    name: &str,
    op: F,
) -> Result<RepositoryMember, ApiError>
where
    F: FnOnce(&mut crate::domain::store::StoredRepository) -> Result<RepositoryMember, ApiError>
        + Send
        + 'static,
{
    let repo_id = repo_id(owner, name);
    let owner = owner.to_string();
    let name = name.to_string();
    match store.inner.as_ref() {
        MetadataStoreInner::Postgres { db, runtime } => {
            let db = Arc::clone(db);
            run_api_db_on(runtime, async move {
                let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                acquire_metadata_write_lock(&tx).await?;
                let row = entities::repository::Entity::find_by_id(repo_id)
                    .one(&tx)
                    .await
                    .map_err(ApiError::internal)?
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                let mut repo = repository_from_model(&tx, row).await?;
                let member = op(&mut repo)?;
                save_repo_mutation(&tx, &repo, &mutation_effects_none()).await?;
                tx.commit().await.map_err(ApiError::internal)?;
                Ok(member)
            })
        }
        #[cfg(any(test, feature = "memory-metadata"))]
        MetadataStoreInner::Memory(_) => store.update(move |catalog| {
            let repo = catalog
                .repositories
                .get_mut(&repo_id)
                .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
            op(repo)
        }),
    }
}

async fn user_by_normalized_email<C>(conn: &C, email: &str) -> Result<Option<UserAccount>, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let normalized = normalize_repository_invite_email(email);
    let users = entities::user::Entity::find()
        .order_by_asc(entities::user::Column::Id)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    users
        .into_iter()
        .map(entities::user::Model::try_into_domain)
        .find(|user| {
            user.as_ref()
                .map(|user| normalize_repository_invite_email(&user.email) == normalized)
                .unwrap_or(true)
        })
        .transpose()
}

fn mutation_effects_none() -> crate::domain::repo_actions::RepoEffects {
    crate::domain::repo_actions::RepoEffects::default()
}
