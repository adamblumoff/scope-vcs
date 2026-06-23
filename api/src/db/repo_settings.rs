use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, entities,
    repo_writes::save_repository_row, repository_from_model, run_api_db_on,
};
use crate::domain::{
    policy::{ScopePath, Visibility, VisibilityRule},
    store::{
        RepoPublicationState, RepoRole, RepoSettings, StoredRepository, pending_import_scope_path,
        repo_id,
    },
};
use crate::error::ApiError;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use std::sync::Arc;

impl MetadataStore {
    pub(crate) fn update_repo_settings(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
        settings: RepoSettings,
        default_visibility: Visibility,
    ) -> Result<StoredRepository, ApiError> {
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
                    let row = entities::repository::Entity::find_by_id(repo_id.clone())
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| {
                            ApiError::not_found(format!("repo {owner}/{name} not found"))
                        })?;
                    let mut repo = repository_from_model(&tx, row).await?;

                    let role = entities::membership::Entity::find()
                        .filter(entities::membership::Column::RepoId.eq(repo_id))
                        .filter(entities::membership::Column::UserId.eq(user_id.clone()))
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .map(|membership| membership.try_into_domain())
                        .transpose()?
                        .map(|membership| membership.role);
                    if role != Some(RepoRole::Owner) {
                        return Err(ApiError::forbidden("owner role required"));
                    }

                    apply_repo_settings(&mut repo, &user_id, settings, default_visibility)?;
                    save_repository_row(&tx, &repo).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(repo)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get(&repo_id)
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                ensure_repo_owner(repo, &user_id)?;

                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .expect("repo was already checked");
                apply_repo_settings(repo, &user_id, settings, default_visibility)?;
                Ok(repo.clone())
            }),
        }
    }
}

fn apply_repo_settings(
    repo: &mut StoredRepository,
    user_id: &str,
    settings: RepoSettings,
    default_visibility: Visibility,
) -> Result<(), ApiError> {
    ensure_repo_owner(repo, user_id)?;
    repo.settings = settings;
    if repo.record.default_visibility != default_visibility {
        preserve_existing_visibility_for_new_default(repo, default_visibility)?;
    }
    repo.record.default_visibility = default_visibility;
    Ok(())
}

fn preserve_existing_visibility_for_new_default(
    repo: &mut StoredRepository,
    default_visibility: Visibility,
) -> Result<(), ApiError> {
    let existing_visibility = existing_repo_paths(repo)?
        .into_iter()
        .map(|path| {
            let visibility = repo.policy.effective_visibility(&path);
            (path, visibility)
        })
        .collect::<Vec<_>>();
    let owner_ids = repo.owner_ids();

    repo.policy.set_default_visibility(default_visibility);
    for (path, visibility) in existing_visibility {
        if repo.policy.effective_visibility(&path) == visibility {
            continue;
        }

        let rule = match visibility {
            Visibility::Public => VisibilityRule::public(path),
            Visibility::Private => VisibilityRule::private(path, owner_ids.clone()),
        };
        repo.policy.add_rule(rule).map_err(ApiError::bad_request)?;
    }
    Ok(())
}

fn existing_repo_paths(repo: &StoredRepository) -> Result<Vec<ScopePath>, ApiError> {
    if repo.record.publication_state == RepoPublicationState::PendingPublish {
        let Some(pending_import) = repo.pending_import.as_ref() else {
            return Ok(Vec::new());
        };
        return pending_import
            .files
            .iter()
            .map(|file| pending_import_scope_path(&file.path).map_err(ApiError::bad_request))
            .collect();
    }

    Ok(repo.live_tree().into_keys().collect())
}

fn ensure_repo_owner(repo: &StoredRepository, user_id: &str) -> Result<(), ApiError> {
    let role = repo
        .memberships
        .iter()
        .find(|membership| membership.user_id == user_id)
        .map(|membership| membership.role);
    if role != Some(RepoRole::Owner) {
        return Err(ApiError::forbidden("owner role required"));
    }
    Ok(())
}
