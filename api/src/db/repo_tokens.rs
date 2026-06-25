use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, encode_json, entities,
    repository_from_model, run_api_db_on,
};
use crate::domain::{
    repo_actions::ensure_repo_member,
    store::{GitCloneToken, RepoPublicationState, repo_id},
};
use crate::error::ApiError;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait, sea_query::Expr};
use std::sync::Arc;

impl MetadataStore {
    pub(crate) fn create_git_clone_token(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
        git_clone_token: GitCloneToken,
    ) -> Result<GitCloneToken, ApiError> {
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
                    let mut repo = repository_from_model(&tx, repo).await?;
                    ensure_repo_member(&repo, &user_id)?;
                    ensure_published_for_clone(&repo)?;
                    append_clone_token(&mut repo.git_clone_tokens, git_clone_token.clone());

                    entities::repository::Entity::update_many()
                        .filter(entities::repository::Column::Id.eq(repo_id))
                        .col_expr(
                            entities::repository::Column::GitCloneTokens,
                            Expr::value(encode_json(&repo.git_clone_tokens)?),
                        )
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(git_clone_token)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                ensure_repo_member(repo, &user_id)?;
                ensure_published_for_clone(repo)?;
                append_clone_token(&mut repo.git_clone_tokens, git_clone_token.clone());
                Ok(git_clone_token)
            }),
        }
    }
}

fn ensure_published_for_clone(
    repo: &crate::domain::store::StoredRepository,
) -> Result<(), ApiError> {
    if repo.record.publication_state == RepoPublicationState::Published {
        Ok(())
    } else {
        Err(ApiError::conflict("repo must be published before cloning"))
    }
}

fn append_clone_token(tokens: &mut Vec<GitCloneToken>, token: GitCloneToken) {
    tokens.push(token);
    tokens.sort_by(|left, right| {
        left.user_id
            .cmp(&right.user_id)
            .then(left.created_at_unix.cmp(&right.created_at_unix))
            .then(left.token_hash.cmp(&right.token_hash))
    });
}
