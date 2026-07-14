use super::{
    MetadataStore, entities,
    projection_encoding::{ProjectionAudience, ProjectionSource},
};
use crate::{
    domain::{
        policy::{Principal, PrincipalKind, ScopePath},
        projection_views::{
            ProjectionViewFile, ProjectionViewFileContent,
            projected_file_contents as domain_projected_file_contents,
            projected_files as domain_projected_files,
        },
        store::{RepositoryActor, StoredRepository},
    },
    error::ApiError,
    persistence::unix_now,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder,
};
use std::sync::Arc;

const PROJECTION_FILE_INSERT_BATCH_SIZE: usize = 1_000;

fn projection_repo_version(repo_version: u64) -> Result<i64, ApiError> {
    i64::try_from(repo_version).map_err(|_| {
        ApiError::internal_message("projection repository version exceeds PostgreSQL bigint range")
    })
}

pub(super) enum ProjectionFileLookup {
    Found(ProjectionViewFileContent),
    Missing,
    NotReady,
}

pub async fn save_live_projection_read_models<C>(
    conn: &C,
    repo: &StoredRepository,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    delete_live_projection_read_models(conn, &repo.record.id).await?;

    let rebuilt_at_unix = unix_now()?;
    for audience in [ProjectionAudience::Private, ProjectionAudience::Public] {
        let files = projected_files_for_audience(repo, audience);
        let file_count = files.len();
        let file_rows = files
            .into_iter()
            .map(|file| {
                entities::projection_file::Model::live(
                    &repo.record.id,
                    repo.record.change_version,
                    audience,
                    file,
                )
                .map(IntoActiveModel::into_active_model)
            })
            .collect::<Result<Vec<_>, ApiError>>()?;
        for batch in file_rows.chunks(PROJECTION_FILE_INSERT_BATCH_SIZE) {
            entities::projection_file::Entity::insert_many(batch.iter().cloned())
                .exec(conn)
                .await
                .map_err(ApiError::internal)?;
        }

        entities::projection_read_model::Model::live(
            &repo.record.id,
            repo.record.change_version,
            audience,
            rebuilt_at_unix,
            file_count,
        )?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;
    }

    Ok(())
}

async fn delete_live_projection_read_models<C>(conn: &C, repo_id: &str) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::projection_file::Entity::delete_many()
        .filter(entities::projection_file::Column::RepoId.eq(repo_id.to_string()))
        .filter(
            entities::projection_file::Column::Source
                .eq(ProjectionSource::Live.as_str().to_string()),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    entities::projection_read_model::Entity::delete_many()
        .filter(entities::projection_read_model::Column::RepoId.eq(repo_id.to_string()))
        .filter(
            entities::projection_read_model::Column::Source
                .eq(ProjectionSource::Live.as_str().to_string()),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

pub(super) async fn load_live_projection_file_count_for_audience<C>(
    conn: &C,
    repo_id: &str,
    repo_version: u64,
    audience: ProjectionAudience,
) -> Result<Option<usize>, ApiError>
where
    C: ConnectionTrait,
{
    let expected_version = projection_repo_version(repo_version)?;
    let Some(model) = entities::projection_read_model::Entity::find()
        .filter(entities::projection_read_model::Column::RepoId.eq(repo_id.to_string()))
        .filter(entities::projection_read_model::Column::RepoVersion.eq(expected_version))
        .filter(
            entities::projection_read_model::Column::Source
                .eq(ProjectionSource::Live.as_str().to_string()),
        )
        .filter(entities::projection_read_model::Column::Audience.eq(audience.as_str().to_string()))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
    else {
        return Ok(None);
    };

    Ok(Some(usize::try_from(model.file_count).map_err(|_| {
        ApiError::internal_message("projection file count cannot be negative")
    })?))
}

pub(super) async fn load_live_projection_file_for_audience<C>(
    conn: &C,
    repo_id: &str,
    repo_version: u64,
    audience: ProjectionAudience,
    path: &ScopePath,
) -> Result<ProjectionFileLookup, ApiError>
where
    C: ConnectionTrait,
{
    let expected_version = projection_repo_version(repo_version)?;
    let read_model_exists = entities::projection_read_model::Entity::find()
        .filter(entities::projection_read_model::Column::RepoId.eq(repo_id.to_string()))
        .filter(entities::projection_read_model::Column::RepoVersion.eq(expected_version))
        .filter(
            entities::projection_read_model::Column::Source
                .eq(ProjectionSource::Live.as_str().to_string()),
        )
        .filter(entities::projection_read_model::Column::Audience.eq(audience.as_str().to_string()))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .is_some();
    if !read_model_exists {
        return Ok(ProjectionFileLookup::NotReady);
    }
    let row = entities::projection_file::Entity::find()
        .filter(entities::projection_file::Column::RepoId.eq(repo_id.to_string()))
        .filter(entities::projection_file::Column::RepoVersion.eq(expected_version))
        .filter(
            entities::projection_file::Column::Source
                .eq(ProjectionSource::Live.as_str().to_string()),
        )
        .filter(entities::projection_file::Column::Audience.eq(audience.as_str().to_string()))
        .filter(
            entities::projection_file::Column::PathKey
                .eq(entities::projection_file::projection_file_path_key(path)),
        )
        .filter(entities::projection_file::Column::Path.eq(path.as_str().to_string()))
        .one(conn)
        .await
        .map_err(ApiError::internal)?;
    match row {
        Some(row) => Ok(ProjectionFileLookup::Found(row.try_into_content()?)),
        None => Ok(ProjectionFileLookup::Missing),
    }
}

pub(super) async fn load_live_projection_files_for_audience<C>(
    conn: &C,
    repo_id: &str,
    repo_version: u64,
    audience: ProjectionAudience,
) -> Result<Option<Vec<ProjectionViewFile>>, ApiError>
where
    C: ConnectionTrait,
{
    let expected_version = projection_repo_version(repo_version)?;
    let Some(model) = entities::projection_read_model::Entity::find()
        .filter(entities::projection_read_model::Column::RepoId.eq(repo_id.to_string()))
        .filter(entities::projection_read_model::Column::RepoVersion.eq(expected_version))
        .filter(
            entities::projection_read_model::Column::Source
                .eq(ProjectionSource::Live.as_str().to_string()),
        )
        .filter(entities::projection_read_model::Column::Audience.eq(audience.as_str().to_string()))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
    else {
        return Ok(None);
    };

    let rows = entities::projection_file::Entity::find()
        .filter(entities::projection_file::Column::RepoId.eq(repo_id.to_string()))
        .filter(entities::projection_file::Column::RepoVersion.eq(expected_version))
        .filter(
            entities::projection_file::Column::Source
                .eq(ProjectionSource::Live.as_str().to_string()),
        )
        .filter(entities::projection_file::Column::Audience.eq(audience.as_str().to_string()))
        .order_by_asc(entities::projection_file::Column::Path)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;

    let expected_file_count = usize::try_from(model.file_count)
        .map_err(|_| ApiError::internal_message("projection file count cannot be negative"))?;
    if rows.len() != expected_file_count {
        return Ok(None);
    }

    let mut files = Vec::with_capacity(rows.len());
    for row in rows {
        let row_path = row.path.clone();
        match row.try_into_view() {
            Ok(file) => files.push(file),
            Err(error) => {
                tracing::warn!(
                    repo_id,
                    path = %row_path,
                    error = %error.message,
                    "ignoring invalid projection read-model row"
                );
                return Ok(None);
            }
        }
    }

    Ok(Some(files))
}

async fn load_live_projection_files<C>(
    conn: &C,
    repo: &StoredRepository,
    principal: &Principal,
) -> Result<Option<Vec<ProjectionViewFile>>, ApiError>
where
    C: ConnectionTrait,
{
    let audience = live_projection_audience(repo, principal);
    load_live_projection_files_for_audience(
        conn,
        &repo.record.id,
        repo.record.change_version,
        audience,
    )
    .await
}

fn projected_files_for_audience(
    repo: &StoredRepository,
    audience: ProjectionAudience,
) -> Vec<ProjectionViewFileContent> {
    let principal = match audience {
        // Current visibility is binary: private readers all see the same file
        // tree. If policy becomes per-user, this audience key must split too.
        ProjectionAudience::Private => Principal {
            id: repo.record.owner_user_id.clone(),
            kind: PrincipalKind::User,
        },
        ProjectionAudience::Public => Principal::public(),
    };
    domain_projected_file_contents(repo, &principal)
}

fn live_projection_audience(repo: &StoredRepository, principal: &Principal) -> ProjectionAudience {
    let access = repo.access_for_principal(principal);
    if access.actor != RepositoryActor::Public && access.can_read_private_files {
        ProjectionAudience::Private
    } else {
        ProjectionAudience::Public
    }
}

impl MetadataStore {
    pub async fn live_projection_files(
        &self,
        repo: &StoredRepository,
        principal: &Principal,
    ) -> Result<Vec<ProjectionViewFile>, ApiError> {
        let db = Arc::clone(&self.db);
        let repo = repo.clone();
        let principal = principal.clone();
        if let Some(files) = load_live_projection_files(db.as_ref(), &repo, &principal).await? {
            return Ok(files);
        }
        Ok(domain_projected_files(&repo, &principal))
    }
}
