use super::{RepositoryStore, acquire_aggregate_lock, entities, git_segments::load_git_pack_spans};
use crate::error::PostgresError;
use scope_domain::{
    content::SourceBlob,
    landing_file::{
        MAX_REPOSITORY_LANDING_FILE_BYTES, REPOSITORY_LANDING_FILE_PATH, RepositoryLandingFile,
        RepositoryLandingFileMutation,
    },
    repository::git::{GitHead, GitPackSpan},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, TransactionTrait,
    sea_query::OnConflict,
};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct RepositoryLandingFileBackfillCandidate {
    pub repo_id: String,
    pub blob: SourceBlob,
    pub git_head: Option<GitHead>,
    pub git_pack_spans: Vec<GitPackSpan>,
}

pub(super) async fn apply_repository_landing_file_mutation<C>(
    conn: &C,
    repo_id: &str,
    mutation: RepositoryLandingFileMutation,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    match mutation {
        RepositoryLandingFileMutation::Unchanged => Ok(()),
        RepositoryLandingFileMutation::Upsert(landing_file) => {
            let model =
                entities::repository_landing_file::Model::from_domain(repo_id, landing_file)?;
            entities::repository_landing_file::Entity::insert(model.into_active_model())
                .on_conflict(
                    OnConflict::column(entities::repository_landing_file::Column::RepoId)
                        .update_columns([
                            entities::repository_landing_file::Column::Path,
                            entities::repository_landing_file::Column::Oid,
                            entities::repository_landing_file::Column::Sha256,
                            entities::repository_landing_file::Column::SizeBytes,
                            entities::repository_landing_file::Column::GitFileMode,
                            entities::repository_landing_file::Column::ContentBytes,
                        ])
                        .to_owned(),
                )
                .exec(conn)
                .await
                .map_err(PostgresError::internal)?;
            Ok(())
        }
        RepositoryLandingFileMutation::Delete => {
            entities::repository_landing_file::Entity::delete_by_id(repo_id.to_string())
                .exec(conn)
                .await
                .map_err(PostgresError::internal)?;
            Ok(())
        }
    }
}

pub(super) async fn repository_landing_file<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<RepositoryLandingFile>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::repository_landing_file::Entity::find_by_id(repo_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .map(entities::repository_landing_file::Model::try_into_domain)
        .transpose()
}

impl RepositoryStore {
    #[cfg(feature = "test-support")]
    pub async fn delete_repository_landing_file_for_tests(
        &self,
        repo_id: &str,
    ) -> Result<(), PostgresError> {
        entities::repository_landing_file::Entity::delete_by_id(repo_id.to_string())
            .exec(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn repository_landing_file_backfill_candidates(
        &self,
    ) -> Result<Vec<RepositoryLandingFileBackfillCandidate>, PostgresError> {
        let existing = entities::repository_landing_file::Entity::find()
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|row| row.repo_id)
            .collect::<BTreeSet<_>>();
        let rows = entities::live_file::Entity::find()
            .filter(entities::live_file::Column::Path.eq(REPOSITORY_LANDING_FILE_PATH))
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        let mut candidates = Vec::new();
        for row in rows {
            if existing.contains(&row.repo_id) {
                continue;
            }
            let blob: SourceBlob =
                serde_json::from_value(row.content).map_err(PostgresError::internal)?;
            if blob.size_bytes > MAX_REPOSITORY_LANDING_FILE_BYTES as u64 {
                continue;
            }
            let git_head = entities::git_head::Entity::find_by_id(&row.repo_id)
                .one(self.db.as_ref())
                .await
                .map_err(PostgresError::internal)?
                .map(entities::git_head::Model::try_into_domain)
                .transpose()?;
            let git_pack_spans = load_git_pack_spans(self.db.as_ref(), &row.repo_id).await?;
            candidates.push(RepositoryLandingFileBackfillCandidate {
                repo_id: row.repo_id,
                blob,
                git_head,
                git_pack_spans,
            });
        }
        Ok(candidates)
    }

    pub async fn store_backfilled_repository_landing_file(
        &self,
        repo_id: &str,
        landing_file: RepositoryLandingFile,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", repo_id).await?;
        let live = entities::live_file::Entity::find_by_id((
            repo_id.to_string(),
            REPOSITORY_LANDING_FILE_PATH.to_string(),
        ))
        .one(&tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| {
            PostgresError::conflict("repository landing file changed during backfill")
        })?;
        let blob: SourceBlob =
            serde_json::from_value(live.content).map_err(PostgresError::internal)?;
        landing_file
            .verify_source(&blob)
            .map_err(PostgresError::internal)?;
        apply_repository_landing_file_mutation(
            &tx,
            repo_id,
            RepositoryLandingFileMutation::Upsert(landing_file),
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)
    }
}
