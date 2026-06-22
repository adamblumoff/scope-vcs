use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, encode_json, entities,
    repository_from_model, run_api_db_on,
};
use crate::domain::{
    policy::{ScopePath, Visibility, VisibilityRule},
    projection::{AuthorVisibility, FileVisibilityChange, LogicalCommit, MixedCommitPolicy},
    store::{
        FirstPushToken, GitPushToken, RepoPublicationState, RepoRole, RepoSettings,
        StoredRepository, repo_id,
    },
};
use crate::error::ApiError;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait, sea_query::Expr};
use std::sync::Arc;

impl MetadataStore {
    pub(crate) fn update_repo_settings(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
        settings: RepoSettings,
    ) -> Result<RepoSettings, ApiError> {
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
                    if entities::repository::Entity::find_by_id(repo_id.clone())
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .is_none()
                    {
                        return Err(ApiError::not_found(format!(
                            "repo {owner}/{name} not found"
                        )));
                    }

                    let role = entities::membership::Entity::find()
                        .filter(entities::membership::Column::RepoId.eq(repo_id.clone()))
                        .filter(entities::membership::Column::UserId.eq(user_id))
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .map(|membership| membership.try_into_domain())
                        .transpose()?
                        .map(|membership| membership.role);
                    if role != Some(RepoRole::Owner) {
                        return Err(ApiError::forbidden("owner role required"));
                    }

                    entities::repository::Entity::update_many()
                        .filter(entities::repository::Column::Id.eq(repo_id))
                        .col_expr(
                            entities::repository::Column::Settings,
                            Expr::value(encode_json(&settings)?),
                        )
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(settings)
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
                repo.settings = settings;
                Ok(settings)
            }),
        }
    }

    pub(crate) fn update_repo_file_visibility(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
        update_paths: Vec<ScopePath>,
        visibility: Visibility,
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
                    let repo = entities::repository::Entity::find_by_id(repo_id.clone())
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| {
                            ApiError::not_found(format!("repo {owner}/{name} not found"))
                        })?;
                    let mut repo = repository_from_model(&tx, repo).await?;
                    apply_repo_file_visibility(&mut repo, &user_id, &update_paths, visibility)?;

                    entities::repository::Entity::update_many()
                        .filter(entities::repository::Column::Id.eq(repo_id))
                        .col_expr(
                            entities::repository::Column::Policy,
                            Expr::value(encode_json(&repo.policy)?),
                        )
                        .col_expr(
                            entities::repository::Column::Graph,
                            Expr::value(encode_json(&repo.graph)?),
                        )
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(repo)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                apply_repo_file_visibility(repo, &user_id, &update_paths, visibility)?;
                Ok(repo.clone())
            }),
        }
    }

    pub(crate) fn regenerate_repo_setup_tokens(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
        first_push_token: FirstPushToken,
        git_push_token: GitPushToken,
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
                    let repo = entities::repository::Entity::find_by_id(repo_id.clone())
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .ok_or_else(|| {
                            ApiError::not_found(format!("repo {owner}/{name} not found"))
                        })?;
                    let mut repo = repository_from_model(&tx, repo).await?;
                    ensure_repo_setup_access(&repo, &user_id)?;

                    entities::repository::Entity::update_many()
                        .filter(entities::repository::Column::Id.eq(repo_id))
                        .col_expr(
                            entities::repository::Column::FirstPushToken,
                            Expr::value(entities::encode_first_push_token(&first_push_token)?),
                        )
                        .col_expr(
                            entities::repository::Column::GitPushToken,
                            Expr::value(encode_json(&git_push_token)?),
                        )
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    tx.commit().await.map_err(ApiError::internal)?;

                    repo.first_push_token = Some(secretless_first_push_token(first_push_token));
                    repo.git_push_token = Some(git_push_token);
                    Ok(repo)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(_) => self.update(move |catalog| {
                let repo = catalog
                    .repositories
                    .get_mut(&repo_id)
                    .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
                ensure_repo_setup_access(repo, &user_id)?;
                repo.first_push_token = Some(secretless_first_push_token(first_push_token));
                repo.git_push_token = Some(git_push_token);
                Ok(repo.clone())
            }),
        }
    }
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

fn ensure_repo_setup_access(repo: &StoredRepository, user_id: &str) -> Result<(), ApiError> {
    let role = repo
        .memberships
        .iter()
        .find(|membership| membership.user_id == user_id)
        .map(|membership| membership.role);
    if role != Some(RepoRole::Owner) {
        return Err(ApiError::not_found(format!(
            "repo {} not found",
            repo.record.id
        )));
    }
    if repo.record.publication_state != RepoPublicationState::PendingFirstPush {
        return Err(ApiError::conflict(
            "setup token is only available before the first push",
        ));
    }
    Ok(())
}

fn secretless_first_push_token(mut token: FirstPushToken) -> FirstPushToken {
    token.secret = None;
    token
}

fn apply_repo_file_visibility(
    repo: &mut StoredRepository,
    user_id: &str,
    update_paths: &[ScopePath],
    visibility: Visibility,
) -> Result<(), ApiError> {
    if update_paths.is_empty() {
        return Err(ApiError::bad_request("at least one file path is required"));
    }
    ensure_repo_owner(repo, user_id)?;
    if visibility == Visibility::Public {
        for update_path in update_paths {
            if !repo.has_file_for_visibility_update(update_path) {
                return Err(ApiError::bad_request(format!(
                    "file {} must be tracked by Git before it can be made public",
                    update_path.as_str()
                )));
            }
        }
    }

    let owner_ids = repo.owner_ids();
    let record_visibility_history =
        repo.record.publication_state == RepoPublicationState::Published;
    let live_tree = if record_visibility_history {
        repo.live_tree()
    } else {
        Default::default()
    };
    let mut visibility_changes = Vec::new();
    for update_path in update_paths {
        let old_visibility = repo.policy.effective_visibility(update_path);
        if record_visibility_history && old_visibility != visibility {
            visibility_changes.push(FileVisibilityChange {
                path: update_path.clone(),
                old_visibility,
                new_visibility: visibility,
                current_content: live_tree.get(update_path).cloned(),
            });
        }
        let rule = match visibility {
            Visibility::Public => VisibilityRule::public(update_path.clone()),
            Visibility::Private => VisibilityRule::private(update_path.clone(), owner_ids.clone()),
        };
        repo.policy.add_rule(rule).map_err(ApiError::bad_request)?;
    }
    if !visibility_changes.is_empty() {
        let parent_ids = repo
            .graph
            .commits
            .last()
            .map(|commit| vec![commit.id.clone()])
            .unwrap_or_default();
        repo.graph.commits.push(LogicalCommit {
            id: format!("rv_visibility_{}", repo.graph.commits.len() + 1),
            parent_ids,
            author_id: user_id.to_string(),
            author_visibility: AuthorVisibility::Visible,
            message: "Update file visibility".to_string(),
            mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
            changes: Vec::new(),
            visibility_changes,
        });
    }
    Ok(())
}
