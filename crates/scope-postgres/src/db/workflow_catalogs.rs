use super::{
    RepositoryStore, acquire_aggregate_lock, begin_metadata_read_snapshot, entities,
    git_segments::load_git_pack_spans, repository_from_model,
};
use crate::error::PostgresError;
use scope_domain::{
    content::SourceBlob,
    repository::Repository,
    repository::git::{GitHead, GitPackSpan},
    runs::catalog::{RepositoryWorkflowCatalog, RepositoryWorkflowFile},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, TransactionTrait, sea_query::OnConflict,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct RepositoryWorkflowCatalogBackfillCandidate {
    pub repo_id: String,
    pub source_change_version: u64,
    pub git_head: GitHead,
    pub git_pack_spans: Vec<GitPackSpan>,
    pub workflow_blobs: Vec<(String, SourceBlob)>,
}

pub struct CurrentRepositoryWorkflowCatalog {
    pub repository: Repository,
    pub catalog: Option<RepositoryWorkflowCatalog>,
}

pub(super) async fn apply_repository_workflow_catalog<C>(
    conn: &C,
    catalog: &RepositoryWorkflowCatalog,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    catalog
        .validate_integrity()
        .map_err(PostgresError::internal)?;
    let header = catalog_header_from_domain(catalog)?;
    let repo_id = header.repo_id.clone();

    entities::repository_workflow_file::Entity::delete_many()
        .filter(entities::repository_workflow_file::Column::RepoId.eq(&repo_id))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    entities::repository_workflow_catalog::Entity::insert(header.into_active_model())
        .on_conflict(
            OnConflict::column(entities::repository_workflow_catalog::Column::RepoId)
                .update_columns([
                    entities::repository_workflow_catalog::Column::SourceHeadOid,
                    entities::repository_workflow_catalog::Column::SourceChangeVersion,
                    entities::repository_workflow_catalog::Column::ConfigurationError,
                ])
                .to_owned(),
        )
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;

    for file in catalog.files().unwrap_or_default() {
        workflow_file_from_domain(&repo_id, file)?
            .into_active_model()
            .insert(conn)
            .await
            .map_err(PostgresError::internal)?;
    }
    Ok(())
}

pub(super) async fn repository_workflow_catalog<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<RepositoryWorkflowCatalog>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(header) = entities::repository_workflow_catalog::Entity::find_by_id(repo_id)
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
    else {
        return Ok(None);
    };
    let files = entities::repository_workflow_file::Entity::find()
        .filter(entities::repository_workflow_file::Column::RepoId.eq(repo_id))
        .order_by_asc(entities::repository_workflow_file::Column::Path)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    catalog_from_rows(header, files).map(Some)
}

impl RepositoryStore {
    pub async fn repository_workflow_catalogs(
        &self,
    ) -> Result<Vec<RepositoryWorkflowCatalog>, PostgresError> {
        load_repository_workflow_catalogs(self.db.as_ref()).await
    }

    pub async fn repository_workflow_catalog(
        &self,
        repo_id: &str,
    ) -> Result<Option<RepositoryWorkflowCatalog>, PostgresError> {
        repository_workflow_catalog(self.db.as_ref(), repo_id).await
    }

    pub async fn current_repository_workflow_catalog(
        &self,
        repo_id: &str,
    ) -> Result<Option<CurrentRepositoryWorkflowCatalog>, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let snapshot = match entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        {
            Some(row) => Some(CurrentRepositoryWorkflowCatalog {
                repository: repository_from_model(&tx, row).await?,
                catalog: repository_workflow_catalog(&tx, repo_id).await?,
            }),
            None => None,
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(snapshot)
    }

    pub async fn repository_workflow_catalog_backfill_candidates(
        &self,
    ) -> Result<Vec<RepositoryWorkflowCatalogBackfillCandidate>, PostgresError> {
        let existing = entities::repository_workflow_catalog::Entity::find()
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|row| row.repo_id)
            .collect::<BTreeSet<_>>();
        let repositories = entities::repository::Entity::find()
            .order_by_asc(entities::repository::Column::Id)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        let mut candidates = Vec::new();
        for repository in repositories {
            if existing.contains(&repository.id) {
                continue;
            }
            let Some(git_head) = entities::git_head::Entity::find_by_id(&repository.id)
                .one(self.db.as_ref())
                .await
                .map_err(PostgresError::internal)?
                .map(entities::git_head::Model::try_into_domain)
                .transpose()?
            else {
                continue;
            };
            let source_change_version = git_head.change_version;
            let git_pack_spans = load_git_pack_spans(self.db.as_ref(), &repository.id).await?;
            let workflow_blobs = current_workflow_blobs(self.db.as_ref(), &repository.id).await?;
            candidates.push(RepositoryWorkflowCatalogBackfillCandidate {
                repo_id: repository.id,
                source_change_version,
                git_head,
                git_pack_spans,
                workflow_blobs: workflow_blobs.into_iter().collect(),
            });
        }
        Ok(candidates)
    }

    pub async fn store_backfilled_repository_workflow_catalog(
        &self,
        catalog: &RepositoryWorkflowCatalog,
    ) -> Result<bool, PostgresError> {
        catalog
            .validate_integrity()
            .map_err(PostgresError::internal)?;
        let repo_id = catalog.repository_id();
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", repo_id).await?;
        if entities::repository_workflow_catalog::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .is_some()
        {
            tx.rollback().await.map_err(PostgresError::internal)?;
            return Ok(false);
        }
        if entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .is_none()
        {
            return Err(PostgresError::not_found(
                "repository disappeared during backfill",
            ));
        }
        let git_head = entities::git_head::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::conflict("repository Git head changed during backfill"))?
            .try_into_domain()?;
        catalog
            .verify_source(repo_id, &git_head.head_oid, git_head.change_version)
            .map_err(|_| {
                PostgresError::conflict(
                    "repository workflow catalog source changed during backfill",
                )
            })?;
        if let Some(files) = catalog.files() {
            verify_current_workflow_blobs(&tx, repo_id, files).await?;
        }
        apply_repository_workflow_catalog(&tx, catalog).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(true)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn delete_repository_workflow_catalog_for_tests(
        &self,
        repo_id: &str,
    ) -> Result<(), PostgresError> {
        entities::repository_workflow_catalog::Entity::delete_by_id(repo_id)
            .exec(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn corrupt_repository_workflow_catalog_source_for_tests(
        &self,
        repo_id: &str,
        source_head_oid: &str,
    ) -> Result<(), PostgresError> {
        let statement = sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE scope_repository_workflow_catalogs
             SET source_head_oid = $2
             WHERE repo_id = $1",
            [repo_id.into(), source_head_oid.into()],
        );
        self.db
            .execute(statement)
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn corrupt_repository_workflow_file_content_for_tests(
        &self,
        repo_id: &str,
        path: &str,
        content_bytes: Vec<u8>,
    ) -> Result<(), PostgresError> {
        let statement = sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE scope_repository_workflow_files
             SET content_bytes = $3,
                 size_bytes = octet_length($3::bytea)
             WHERE repo_id = $1 AND path = $2",
            [repo_id.into(), path.into(), content_bytes.into()],
        );
        self.db
            .execute(statement)
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }
}

pub(super) async fn load_repository_workflow_catalogs(
    db: &DatabaseConnection,
) -> Result<Vec<RepositoryWorkflowCatalog>, PostgresError> {
    let tx = begin_metadata_read_snapshot(db).await?;
    let headers = entities::repository_workflow_catalog::Entity::find()
        .order_by_asc(entities::repository_workflow_catalog::Column::RepoId)
        .all(&tx)
        .await
        .map_err(PostgresError::internal)?;
    let mut catalogs = Vec::with_capacity(headers.len());
    for header in headers {
        let files = entities::repository_workflow_file::Entity::find()
            .filter(entities::repository_workflow_file::Column::RepoId.eq(header.repo_id.clone()))
            .order_by_asc(entities::repository_workflow_file::Column::Path)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        catalogs.push(catalog_from_rows(header, files)?);
    }
    tx.commit().await.map_err(PostgresError::internal)?;
    Ok(catalogs)
}

fn catalog_header_from_domain(
    catalog: &RepositoryWorkflowCatalog,
) -> Result<entities::repository_workflow_catalog::Model, PostgresError> {
    Ok(entities::repository_workflow_catalog::Model {
        repo_id: catalog.repository_id().to_string(),
        source_head_oid: catalog.source_head_oid().to_string(),
        source_change_version: i64::try_from(catalog.source_change_version()).map_err(|_| {
            PostgresError::internal_message(
                "repository workflow catalog change version exceeds PostgreSQL bigint range",
            )
        })?,
        configuration_error: catalog.configuration_error().map(str::to_string),
    })
}

fn workflow_file_from_domain(
    repo_id: &str,
    file: &RepositoryWorkflowFile,
) -> Result<entities::repository_workflow_file::Model, PostgresError> {
    file.validate_integrity().map_err(PostgresError::internal)?;
    Ok(entities::repository_workflow_file::Model {
        repo_id: repo_id.to_string(),
        path: file.path().as_str().to_string(),
        oid: file.oid().to_string(),
        size_bytes: i64::try_from(file.size_bytes()).map_err(|_| {
            PostgresError::internal_message(
                "repository workflow file size exceeds PostgreSQL bigint range",
            )
        })?,
        git_file_mode: file.git_file_mode().to_string(),
        content_bytes: file.content_bytes().to_vec(),
    })
}

fn catalog_from_rows(
    header: entities::repository_workflow_catalog::Model,
    files: Vec<entities::repository_workflow_file::Model>,
) -> Result<RepositoryWorkflowCatalog, PostgresError> {
    let source_change_version = u64::try_from(header.source_change_version).map_err(|_| {
        PostgresError::internal_message(
            "repository workflow catalog change version cannot be negative",
        )
    })?;
    match header.configuration_error {
        Some(error) if files.is_empty() => RepositoryWorkflowCatalog::rejected(
            header.repo_id,
            header.source_head_oid,
            source_change_version,
            error,
        )
        .map_err(PostgresError::internal),
        Some(_) => Err(PostgresError::internal_message(
            "rejected repository workflow catalog contains files",
        )),
        None => RepositoryWorkflowCatalog::captured(
            header.repo_id,
            header.source_head_oid,
            source_change_version,
            files
                .into_iter()
                .map(workflow_file_from_row)
                .collect::<Result<Vec<_>, _>>()?,
        )
        .map_err(PostgresError::internal),
    }
}

fn workflow_file_from_row(
    row: entities::repository_workflow_file::Model,
) -> Result<RepositoryWorkflowFile, PostgresError> {
    RepositoryWorkflowFile::new(
        row.path,
        row.oid,
        u64::try_from(row.size_bytes).map_err(|_| {
            PostgresError::internal_message("repository workflow file size cannot be negative")
        })?,
        row.git_file_mode,
        row.content_bytes,
    )
    .map_err(PostgresError::internal)
}

async fn current_workflow_blobs<C>(
    conn: &C,
    repo_id: &str,
) -> Result<BTreeMap<String, SourceBlob>, PostgresError>
where
    C: ConnectionTrait,
{
    let rows = entities::live_file::Entity::find()
        .filter(entities::live_file::Column::RepoId.eq(repo_id))
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    let mut blobs = BTreeMap::new();
    for row in rows {
        if !row.path.starts_with("/.scope/runs/") {
            continue;
        }
        blobs.insert(
            row.path,
            serde_json::from_value(row.content).map_err(PostgresError::internal)?,
        );
    }
    Ok(blobs)
}

async fn verify_current_workflow_blobs<C>(
    conn: &C,
    repo_id: &str,
    files: &[RepositoryWorkflowFile],
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let blobs = current_workflow_blobs(conn, repo_id).await?;
    if blobs.len() != files.len() {
        return Err(PostgresError::conflict(
            "repository workflows changed during catalog backfill",
        ));
    }
    for file in files {
        let Some(blob) = blobs.get(file.path().as_str()) else {
            return Err(PostgresError::conflict(
                "repository workflows changed during catalog backfill",
            ));
        };
        if file.verify_source(blob).is_err() {
            return Err(PostgresError::conflict(
                "repository workflows changed during catalog backfill",
            ));
        }
    }
    Ok(())
}
