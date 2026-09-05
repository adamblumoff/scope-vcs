use super::{
    RepositoryStore, begin_metadata_read_snapshot, entities, git_segments::load_git_pack_spans,
    history_rows::RepositoryHistory, repository_rows::RepositoryFactRows,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use std::collections::BTreeMap;
use {
    crate::error::PostgresError,
    scope_domain::{
        projection::SourceGraph,
        repo_config::RepoConfig,
        repository::access::RepositoryAccess,
        repository::git::{GitHead, GitPackSpan},
        repository::{RepoLifecycleState, RepositoryIncarnation, repo_id},
    },
};

#[derive(Clone, Debug)]
pub struct GitPushContext {
    pub repo_id: String,
    pub incarnation: RepositoryIncarnation,
    pub owner_user_id: String,
    pub lifecycle_state: RepoLifecycleState,
    pub access: RepositoryAccess,
    pub repo_config: RepoConfig,
    pub git_head: Option<GitHead>,
    pub git_pack_spans: Vec<GitPackSpan>,
    pub change_version: u64,
}

impl RepositoryStore {
    pub async fn run_repository_incarnation(
        &self,
        run_id: &str,
        expected_repository_id: &str,
    ) -> Result<Option<RepositoryIncarnation>, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let Some(run) = entities::run::Entity::find_by_id(run_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        if run.repo_id != expected_repository_id {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        }
        let repository = entities::repository::Entity::find_by_id(&run.repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        repository
            .map(|row| RepositoryIncarnation::new(row.id, row.incarnation_id))
            .transpose()
            .map_err(PostgresError::internal)
    }

    pub async fn git_push_context(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
    ) -> Result<Option<GitPushContext>, PostgresError> {
        let id = repo_id(owner, name);
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let context = git_push_context_for_id(&tx, &id, user_id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(context)
    }
}

pub(super) async fn git_push_context_for_id<C: ConnectionTrait>(
    conn: &C,
    id: &str,
    user_id: &str,
) -> Result<Option<GitPushContext>, PostgresError> {
    let id = id.to_string();
    let Some(repo_row) = entities::repository::Entity::find_by_id(id.clone())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
    else {
        return Ok(None);
    };
    let head = entities::git_head::Entity::find_by_id(id.clone())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .map(entities::git_head::Model::try_into_domain)
        .transpose()?;
    let pack_spans = load_git_pack_spans(conn, &id).await?;
    let members = entities::repository_member::Entity::find()
        .filter(entities::repository_member::Column::RepoId.eq(id.clone()))
        .filter(entities::repository_member::Column::UserId.eq(user_id.to_string()))
        .all(conn)
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
        incarnation: repo.incarnation(),
        owner_user_id: repo.record.owner_user_id.clone(),
        lifecycle_state: repo.record.lifecycle_state,
        access: repo.access_for_user_id(user_id),
        repo_config: repo.repo_config,
        git_head: repo.git_head,
        git_pack_spans: repo.git_pack_spans,
        change_version: repo.record.change_version,
    };
    Ok(Some(context))
}
