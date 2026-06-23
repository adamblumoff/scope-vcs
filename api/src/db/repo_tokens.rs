use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, encode_json, entities,
    repository_from_model, run_api_db_on,
};
use crate::domain::{
    repo_actions::ensure_repo_owner,
    store::{GitPushToken, repo_id},
};
use crate::error::ApiError;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait, sea_query::Expr};
use std::sync::Arc;

impl MetadataStore {
    pub(crate) fn regenerate_git_push_token(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
        git_push_token: GitPushToken,
    ) -> Result<GitPushToken, ApiError> {
        let repo_id = repo_id(owner, name);
        let owner = owner.to_string();
        let name = name.to_string();
        let user_id = user_id.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let repo = entities::repository::Entity::find_by_id(repo_id.clone())
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| {
                            ApiError::not_found(format!("repo {owner}/{name} not found"))
                        })?;
                    let repo = repository_from_model(&tx, repo).await?;
                    ensure_repo_owner(&repo, &user_id)?;

                    entities::repository::Entity::update_many()
                        .filter(entities::repository::Column::Id.eq(repo_id))
                        .col_expr(
                            entities::repository::Column::GitPushToken,
                            Expr::value(encode_json(&git_push_token)?),
                        )
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(git_push_token)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                ensure_repo_owner(repo, &user_id)?;
                repo.git_push_token = Some(git_push_token.clone());
                Ok(git_push_token)
            }),
        }
    }
}
