use super::{
    GeneratedIdSource, RepositoryStore, acquire_aggregate_lock, entities,
    repo_effects::save_repo_mutation, repository_from_model,
};
use crate::error::PostgresError;
use scope_domain::{
    policy::{ScopePath, Visibility},
    repo_actions::set_visibility,
    repository::{Repository, repo_id},
};
use sea_orm::{EntityTrait, TransactionTrait};
use std::sync::Arc;

pub struct UpdateRepoFileVisibilityCommand {
    pub owner: String,
    pub name: String,
    pub user_id: String,
    pub update_paths: Vec<ScopePath>,
    pub visibility: Visibility,
    pub now_unix: u64,
}

impl RepositoryStore {
    pub async fn update_repo_file_visibility(
        &self,
        command: UpdateRepoFileVisibilityCommand,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<Repository, PostgresError> {
        let UpdateRepoFileVisibilityCommand {
            owner,
            name,
            user_id,
            update_paths,
            visibility,
            now_unix,
        } = command;
        let repo_id = repo_id(&owner, &name);
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let repo = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
        let mut repo = repository_from_model(&tx, repo).await?;
        let before = repo.clone();
        let mutation = set_visibility(&mut repo, &user_id, &update_paths, visibility)?;
        save_repo_mutation(
            &tx,
            &before,
            &repo,
            &mutation.effects,
            now_unix,
            generated_ids,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(repo)
    }
}
