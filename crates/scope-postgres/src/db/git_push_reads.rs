use super::{
    RepositoryStore, begin_metadata_read_snapshot, entities, history_rows::RepositoryHistory,
    repository_rows::RepositoryFactRows,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use std::collections::BTreeMap;
use {
    crate::error::PostgresError,
    scope_domain::{
        projection::SourceGraph,
        repo_config::RepoConfig,
        store::{GitHead, GitPackSpan, RepoLifecycleState, RepositoryAccess, repo_id},
    },
};

#[derive(Clone, Debug)]
pub struct GitPushContext {
    pub repo_id: String,
    pub owner_user_id: String,
    pub lifecycle_state: RepoLifecycleState,
    pub access: RepositoryAccess,
    pub repo_config: RepoConfig,
    pub git_head: Option<GitHead>,
    pub git_pack_spans: Vec<GitPackSpan>,
    pub change_version: u64,
}

impl RepositoryStore {
    pub async fn git_push_context(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
    ) -> Result<Option<GitPushContext>, PostgresError> {
        let id = repo_id(owner, name);
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let Some(repo_row) = entities::repository::Entity::find_by_id(id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        let head = entities::git_head::Entity::find_by_id(id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .map(entities::git_head::Model::try_into_domain)
            .transpose()?;
        let pack_spans = entities::git_pack_span::Entity::find()
            .filter(entities::git_pack_span::Column::RepoId.eq(id.clone()))
            .order_by_asc(entities::git_pack_span::Column::FirstSequence)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::git_pack_span::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        let members = entities::repository_member::Entity::find()
            .filter(entities::repository_member::Column::RepoId.eq(id.clone()))
            .filter(entities::repository_member::Column::UserId.eq(user_id.to_string()))
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::repository_member::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        let repo = repo_row.try_into_domain(
            RepositoryFactRows {
                git_head: head,
                git_pack_spans: pack_spans,
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
                visibility_change_sets: Vec::new(),
                live_files: BTreeMap::new(),
            },
        )?;
        let context = GitPushContext {
            repo_id: id,
            owner_user_id: repo.record.owner_user_id.clone(),
            lifecycle_state: repo.record.lifecycle_state,
            access: repo.access_for_user_id(user_id),
            repo_config: repo.repo_config,
            git_head: repo.git_head,
            git_pack_spans: repo.git_pack_spans,
            change_version: repo.record.change_version,
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(context))
    }
}
