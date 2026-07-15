use super::{
    MetadataStore, begin_metadata_read_snapshot, entities, history_rows::RepositoryHistory,
    repository_rows::RepositoryFactRows,
};
use crate::{
    domain::{
        projection::SourceGraph,
        repo_config::RepoConfig,
        store::{GitHead, RepoPublicationState, RepositoryAccess, repo_id},
    },
    error::ApiError,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct GitPushContext {
    pub repo_id: String,
    pub owner_user_id: String,
    pub publication_state: RepoPublicationState,
    pub access: RepositoryAccess,
    pub repo_config: RepoConfig,
    pub git_head: Option<GitHead>,
    pub change_version: u64,
}

impl MetadataStore {
    pub async fn git_push_context(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
    ) -> Result<Option<GitPushContext>, ApiError> {
        let id = repo_id(owner, name);
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let Some(repo_row) = entities::repository::Entity::find_by_id(id.clone())
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
        else {
            tx.commit().await.map_err(ApiError::internal)?;
            return Ok(None);
        };
        let head = entities::git_head::Entity::find_by_id(id.clone())
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
            .map(entities::git_head::Model::try_into_domain)
            .transpose()?;
        let members = entities::repository_member::Entity::find()
            .filter(entities::repository_member::Column::RepoId.eq(id.clone()))
            .filter(entities::repository_member::Column::UserId.eq(user_id.to_string()))
            .all(&tx)
            .await
            .map_err(ApiError::internal)?
            .into_iter()
            .map(entities::repository_member::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        let repo = repo_row.try_into_domain(
            RepositoryFactRows {
                git_head: head,
                ..Default::default()
            }
            .into_facts(),
            members,
            Vec::new(),
            RepositoryHistory {
                graph: SourceGraph {
                    repo_id: id.clone(),
                    commits: Vec::new(),
                },
                visibility_events: Vec::new(),
                live_files: BTreeMap::new(),
            },
        )?;
        let context = GitPushContext {
            repo_id: id,
            owner_user_id: repo.record.owner_user_id.clone(),
            publication_state: repo.record.publication_state,
            access: repo.access_for_user_id(user_id),
            repo_config: repo.repo_config,
            git_head: repo.git_head,
            change_version: repo.record.change_version,
        };
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(Some(context))
    }
}
