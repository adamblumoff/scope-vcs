use super::{
    GeneratedIdSource, RepositoryStore, acquire_aggregate_lock, auth::load_user_by_id, entities,
    repo_effects::save_repo_mutation, repository_from_model,
};
use crate::error::PostgresError;
use scope_domain::{
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

pub struct UpdateRepositoryMemberPermissionsCommand {
    pub owner: String,
    pub name: String,
    pub owner_user_id: String,
    pub member_user_id: String,
    pub permissions: RepositoryMemberPermissions,
    pub now_unix: u64,
}

impl RepositoryStore {
    pub async fn repository_collaboration(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<(StoredRepository, BTreeMap<String, UserAccount>)>, PostgresError> {
        let Some(row) = entities::repository::Entity::find_by_id(repo_id(owner, name))
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
        else {
            return Ok(None);
        };
        let repo = repository_from_model(self.db.as_ref(), row).await?;
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
                .map_err(PostgresError::internal)?
                .into_iter()
                .map(|row| {
                    let user = row.try_into_domain()?;
                    Ok((user.id.clone(), user))
                })
                .collect::<Result<_, PostgresError>>()?
        };
        Ok(Some((repo, users)))
    }

    pub async fn user(&self, user_id: &str) -> Result<UserAccount, PostgresError> {
        load_user_by_id(self.db.as_ref(), user_id).await
    }

    pub async fn create_repository_invite(
        &self,
        command: CreateRepositoryInviteMutation,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<RepositoryInvite, PostgresError> {
        let now_unix = command.now_unix;
        let repo_id = repo_id(&command.owner, &command.name);
        let owner_name = command.owner.clone();
        let name = command.name.clone();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let row = entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| {
                PostgresError::not_found(format!("repo {owner_name}/{name} not found"))
            })?;
        let mut repo = repository_from_model(&tx, row).await?;
        let before = repo.clone();
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
        save_repo_mutation(
            &tx,
            &before,
            &repo,
            &mutation_effects_none(),
            now_unix,
            generated_ids,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn update_repository_member_permissions(
        &self,
        command: UpdateRepositoryMemberPermissionsCommand,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<RepositoryMember, PostgresError> {
        let UpdateRepositoryMemberPermissionsCommand {
            owner,
            name,
            owner_user_id,
            member_user_id,
            permissions,
            now_unix,
        } = command;
        mutate_repository_collaboration(self, &owner, &name, now_unix, generated_ids, move |repo| {
            update_repository_member_permissions(
                repo,
                &owner_user_id,
                &member_user_id,
                permissions,
                now_unix,
            )
            .map_err(PostgresError::from)
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
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<RepositoryInvite, PostgresError> {
        let owner_user_id = owner_user_id.to_string();
        let invite_id = invite_id.to_string();
        mutate_repository_collaboration(self, owner, name, now_unix, generated_ids, move |repo| {
            revoke_repository_invite(repo, &owner_user_id, &invite_id, now_unix)
                .map_err(PostgresError::from)
        })
        .await
    }

    pub async fn remove_repository_member(
        &self,
        owner: &str,
        name: &str,
        owner_user_id: &str,
        member_user_id: &str,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<RepositoryMember, PostgresError> {
        let repo_id = repo_id(owner, name);
        let owner = owner.to_string();
        let name = name.to_string();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let row = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
        let mut repo = repository_from_model(&tx, row).await?;
        let before = repo.clone();
        let removed = remove_repository_member(&mut repo, owner_user_id, member_user_id)
            .map_err(PostgresError::from)?;
        save_repo_mutation(
            &tx,
            &before,
            &repo,
            &mutation_effects_none(),
            now_unix,
            generated_ids,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(removed)
    }

    pub async fn repository_invite_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<(scope_domain::store::StoredRepository, RepositoryInvite), PostgresError> {
        let invite = entities::repository_invite::Entity::find()
            .filter(entities::repository_invite::Column::TokenHash.eq(token_hash.to_string()))
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("repository invite not found"))?;
        let repo_row = entities::repository::Entity::find_by_id(invite.repo_id.clone())
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("repository invite repo is missing"))?;
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
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<(scope_domain::store::StoredRepository, RepositoryMember), PostgresError> {
        let token_hash = token_hash.to_string();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository-invite-token", &token_hash).await?;
        let invite = entities::repository_invite::Entity::find()
            .filter(entities::repository_invite::Column::TokenHash.eq(token_hash.clone()))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("repository invite not found"))?;
        acquire_aggregate_lock(&tx, "repository", &invite.repo_id).await?;
        let row = entities::repository::Entity::find_by_id(invite.repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("repository invite not found"))?;
        let mut repo = repository_from_model(&tx, row).await?;
        let before = repo.clone();
        let outcome = accept_repository_invite(&mut repo, &user, &token_hash, now_unix)?;
        save_repo_mutation(
            &tx,
            &before,
            &repo,
            &mutation_effects_none(),
            now_unix,
            generated_ids,
        )
        .await?;
        let result = match outcome {
            AcceptRepositoryInviteOutcome::Accepted(member) => Ok((repo, member)),
            AcceptRepositoryInviteOutcome::Expired => {
                Err(PostgresError::conflict("repository invite expired"))
            }
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        result
    }
}

async fn mutate_repository_collaboration<T, F>(
    store: &RepositoryStore,
    owner: &str,
    name: &str,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
    op: F,
) -> Result<T, PostgresError>
where
    T: Send + 'static,
    F: FnOnce(&mut StoredRepository) -> Result<T, PostgresError> + Send + 'static,
{
    let repo_id = repo_id(owner, name);
    let owner = owner.to_string();
    let name = name.to_string();
    let db = Arc::clone(&store.db);
    let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
    acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
    let row = entities::repository::Entity::find_by_id(repo_id)
        .one(&tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
    let mut repo = repository_from_model(&tx, row).await?;
    let before = repo.clone();
    let result = op(&mut repo)?;
    save_repo_mutation(
        &tx,
        &before,
        &repo,
        &mutation_effects_none(),
        now_unix,
        generated_ids,
    )
    .await?;
    tx.commit().await.map_err(PostgresError::internal)?;
    Ok(result)
}

async fn user_by_normalized_email<C>(
    conn: &C,
    email: &str,
) -> Result<Option<UserAccount>, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let normalized = normalize_repository_invite_email(email);
    entities::user::Entity::find()
        .filter(entities::user::Column::Email.eq(normalized))
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .map(entities::user::Model::try_into_domain)
        .transpose()
}

fn mutation_effects_none() -> scope_domain::repo_actions::RepoEffects {
    scope_domain::repo_actions::RepoEffects::default()
}
