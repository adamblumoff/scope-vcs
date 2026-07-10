use super::{
    MetadataStore, acquire_metadata_write_lock, auth::load_user_by_id, entities,
    repo_effects::save_repo_mutation, repository_from_model,
};
use crate::domain::{
    repo_collaboration::{
        AcceptRepositoryInviteOutcome, CreateRepositoryInviteCommand, accept_repository_invite,
        create_or_refresh_repository_invite, remove_repository_member, revoke_repository_invite,
        update_repository_member_permissions,
    },
    store::{
        RepositoryInvite, RepositoryMember, RepositoryMemberPermissions, StoredRepository,
        UserAccount, normalize_repository_invite_email, repo_id,
    },
};
use crate::error::ApiError;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use std::{collections::BTreeMap, sync::Arc};

pub struct CreateRepositoryInviteMutation {
    pub owner: String,
    pub name: String,
    pub owner_user: UserAccount,
    pub invited_email: String,
    pub permissions: RepositoryMemberPermissions,
    pub invite_id: String,
    pub token_hash: String,
    pub now_unix: u64,
}

impl MetadataStore {
    pub async fn repository_collaboration(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<(StoredRepository, BTreeMap<String, UserAccount>)>, ApiError> {
        let Some(repo) = self.repository(owner, name).await? else {
            return Ok(None);
        };
        let user_ids = repo
            .members
            .iter()
            .map(|member| member.user_id.clone())
            .collect::<Vec<_>>();
        let users = if user_ids.is_empty() {
            BTreeMap::new()
        } else {
            entities::user::Entity::find()
                .filter(entities::user::Column::Id.is_in(user_ids))
                .all(self.db.as_ref())
                .await
                .map_err(ApiError::internal)?
                .into_iter()
                .map(|row| {
                    let user = row.try_into_domain()?;
                    Ok((user.id.clone(), user))
                })
                .collect::<Result<_, ApiError>>()?
        };
        Ok(Some((repo, users)))
    }

    pub async fn user(&self, user_id: &str) -> Result<UserAccount, ApiError> {
        load_user_by_id(self.db.as_ref(), user_id).await
    }

    pub async fn create_repository_invite(
        &self,
        command: CreateRepositoryInviteMutation,
    ) -> Result<RepositoryInvite, ApiError> {
        let repo_id = repo_id(&command.owner, &command.name);
        let owner_name = command.owner.clone();
        let name = command.name.clone();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_metadata_write_lock(&tx).await?;
        let row = entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found(format!("repo {owner_name}/{name} not found")))?;
        let mut repo = repository_from_model(&tx, row).await?;
        let invitee = user_by_normalized_email(&tx, &command.invited_email).await?;
        let mutation = create_or_refresh_repository_invite(
            &mut repo,
            CreateRepositoryInviteCommand {
                id: command.invite_id,
                owner: &command.owner_user,
                invited_email: command.invited_email,
                invitee: invitee.as_ref(),
                permissions: command.permissions,
                token_hash: command.token_hash,
                now_unix: command.now_unix,
            },
        )?;
        save_repo_mutation(&tx, &repo, &mutation_effects_none()).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn update_repository_member_permissions(
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
        mutate_repository_collaboration(self, owner, name, move |repo| {
            update_repository_member_permissions(
                repo,
                &owner_user_id,
                &member_user_id,
                permissions,
                now_unix,
            )
        })
        .await
    }

    pub async fn revoke_repository_invite(
        &self,
        owner: &str,
        name: &str,
        owner_user_id: &str,
        invite_id: &str,
        now_unix: u64,
    ) -> Result<RepositoryInvite, ApiError> {
        let owner_user_id = owner_user_id.to_string();
        let invite_id = invite_id.to_string();
        mutate_repository_collaboration(self, owner, name, move |repo| {
            revoke_repository_invite(repo, &owner_user_id, &invite_id, now_unix)
        })
        .await
    }

    pub async fn remove_repository_member(
        &self,
        owner: &str,
        name: &str,
        owner_user_id: &str,
        member_user_id: &str,
    ) -> Result<RepositoryMember, ApiError> {
        let owner_user_id = owner_user_id.to_string();
        let member_user_id = member_user_id.to_string();
        mutate_repository_collaboration(self, owner, name, move |repo| {
            remove_repository_member(repo, &owner_user_id, &member_user_id)
        })
        .await
    }

    pub async fn repository_invite_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<(crate::domain::store::StoredRepository, RepositoryInvite), ApiError> {
        let invite = entities::repository_invite::Entity::find()
            .filter(entities::repository_invite::Column::TokenHash.eq(token_hash.to_string()))
            .one(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found("repository invite not found"))?;
        let repo_row = entities::repository::Entity::find_by_id(invite.repo_id.clone())
            .one(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::internal_message("repository invite repo is missing"))?;
        Ok((
            repository_from_model(self.db.as_ref(), repo_row).await?,
            invite.try_into_domain()?,
        ))
    }

    pub async fn accept_repository_invite(
        &self,
        token_hash: &str,
        user: UserAccount,
        now_unix: u64,
    ) -> Result<(crate::domain::store::StoredRepository, RepositoryMember), ApiError> {
        let token_hash = token_hash.to_string();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_metadata_write_lock(&tx).await?;
        let invite = entities::repository_invite::Entity::find()
            .filter(entities::repository_invite::Column::TokenHash.eq(token_hash.clone()))
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
        let outcome = accept_repository_invite(&mut repo, &user, &token_hash, now_unix)?;
        save_repo_mutation(&tx, &repo, &mutation_effects_none()).await?;
        let result = match outcome {
            AcceptRepositoryInviteOutcome::Accepted(member) => Ok((repo, member)),
            AcceptRepositoryInviteOutcome::Expired => {
                Err(ApiError::conflict("repository invite expired"))
            }
        };
        tx.commit().await.map_err(ApiError::internal)?;
        result
    }
}

async fn mutate_repository_collaboration<T, F>(
    store: &MetadataStore,
    owner: &str,
    name: &str,
    op: F,
) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&mut StoredRepository) -> Result<T, ApiError> + Send + 'static,
{
    let repo_id = repo_id(owner, name);
    let owner = owner.to_string();
    let name = name.to_string();
    let db = Arc::clone(&store.db);
    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
    acquire_metadata_write_lock(&tx).await?;
    let row = entities::repository::Entity::find_by_id(repo_id)
        .one(&tx)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
    let mut repo = repository_from_model(&tx, row).await?;
    let result = op(&mut repo)?;
    save_repo_mutation(&tx, &repo, &mutation_effects_none()).await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(result)
}

async fn user_by_normalized_email<C>(conn: &C, email: &str) -> Result<Option<UserAccount>, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let normalized = normalize_repository_invite_email(email);
    entities::user::Entity::find()
        .filter(entities::user::Column::Email.eq(normalized))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::user::Model::try_into_domain)
        .transpose()
}

fn mutation_effects_none() -> crate::domain::repo_actions::RepoEffects {
    crate::domain::repo_actions::RepoEffects::default()
}
