use super::{MetadataStore, MetadataStoreInner, entities, run_api_db_on};
use crate::{
    domain::{
        policy::{Principal, PrincipalKind},
        projection_views::{ProjectionViewFile, projected_files as domain_projected_files},
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

const LIVE_SOURCE: &str = "live";
const PRIVATE_AUDIENCE: &str = "private";
const PUBLIC_AUDIENCE: &str = "public";
const PROJECTION_FILE_INSERT_BATCH_SIZE: usize = 1_000;

pub async fn save_live_projection_read_models<C>(
    conn: &C,
    repo: &StoredRepository,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    delete_live_projection_read_models(conn, &repo.record.id).await?;

    let rebuilt_at_unix = unix_now()?;
    for audience in [PRIVATE_AUDIENCE, PUBLIC_AUDIENCE] {
        let files = projected_files_for_audience(repo, audience);
        entities::projection_read_model::Model::live(
            &repo.record.id,
            repo.record.change_version,
            audience,
            rebuilt_at_unix,
            files.len(),
        )
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;

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
        if !file_rows.is_empty() {
            for batch in file_rows.chunks(PROJECTION_FILE_INSERT_BATCH_SIZE) {
                entities::projection_file::Entity::insert_many(batch.iter().cloned())
                    .exec(conn)
                    .await
                    .map_err(ApiError::internal)?;
            }
        }
    }

    Ok(())
}

async fn delete_live_projection_read_models<C>(conn: &C, repo_id: &str) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::projection_file::Entity::delete_many()
        .filter(entities::projection_file::Column::RepoId.eq(repo_id.to_string()))
        .filter(entities::projection_file::Column::Source.eq(LIVE_SOURCE.to_string()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    entities::projection_read_model::Entity::delete_many()
        .filter(entities::projection_read_model::Column::RepoId.eq(repo_id.to_string()))
        .filter(entities::projection_read_model::Column::Source.eq(LIVE_SOURCE.to_string()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
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
    let expected_version = repo.record.change_version.min(i64::MAX as u64) as i64;
    let Some(model) = entities::projection_read_model::Entity::find()
        .filter(entities::projection_read_model::Column::RepoId.eq(repo.record.id.clone()))
        .filter(entities::projection_read_model::Column::RepoVersion.eq(expected_version))
        .filter(entities::projection_read_model::Column::Source.eq(LIVE_SOURCE.to_string()))
        .filter(entities::projection_read_model::Column::Audience.eq(audience.to_string()))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
    else {
        return Ok(None);
    };

    let rows = entities::projection_file::Entity::find()
        .filter(entities::projection_file::Column::RepoId.eq(repo.record.id.clone()))
        .filter(entities::projection_file::Column::RepoVersion.eq(expected_version))
        .filter(entities::projection_file::Column::Source.eq(LIVE_SOURCE.to_string()))
        .filter(entities::projection_file::Column::Audience.eq(audience.to_string()))
        .order_by_asc(entities::projection_file::Column::Path)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;

    if rows.len() != model.file_count.max(0) as usize {
        return Ok(None);
    }

    let mut files = Vec::with_capacity(rows.len());
    for row in rows {
        let row_path = row.path.clone();
        match row.try_into_view() {
            Ok(file) => files.push(file),
            Err(error) => {
                tracing::warn!(
                    repo_id = %repo.record.id,
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

fn projected_files_for_audience(
    repo: &StoredRepository,
    audience: &str,
) -> Vec<ProjectionViewFile> {
    let principal = match audience {
        // Current visibility is binary: private readers all see the same file
        // tree. If policy becomes per-user, this audience key must split too.
        PRIVATE_AUDIENCE => Principal {
            id: repo.record.owner_user_id.clone(),
            kind: PrincipalKind::User,
        },
        PUBLIC_AUDIENCE => Principal::public(),
        _ => unreachable!("projection read-model audience is fixed"),
    };
    domain_projected_files(repo, &principal)
}

fn live_projection_audience(repo: &StoredRepository, principal: &Principal) -> &'static str {
    let access = repo.access_for_principal(principal);
    if access.actor != RepositoryActor::Public && access.can_read_private_files {
        PRIVATE_AUDIENCE
    } else {
        PUBLIC_AUDIENCE
    }
}

impl MetadataStore {
    pub fn live_projection_files(
        &self,
        repo: &StoredRepository,
        principal: &Principal,
    ) -> Result<Vec<ProjectionViewFile>, ApiError> {
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                let repo = repo.clone();
                let principal = principal.clone();
                run_api_db_on(runtime, async move {
                    if let Some(files) =
                        load_live_projection_files(db.as_ref(), &repo, &principal).await?
                    {
                        return Ok(files);
                    }
                    Ok(domain_projected_files(&repo, &principal))
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(_) => Ok(domain_projected_files(repo, principal)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        policy::{Policy, ScopePath, Visibility},
        projection::{AuthorVisibility, FileChange, LogicalCommit, SourceGraph},
        store::{RepoPublicationState, RepoRecord, RepoSettings},
    };

    #[test]
    fn private_audience_read_model_keeps_private_files() {
        let repo = read_model_repo();
        let files = projected_files_for_audience(&repo, PRIVATE_AUDIENCE);
        let paths = files
            .into_iter()
            .map(|file| file.path.as_str().to_string())
            .collect::<Vec<_>>();

        assert_eq!(paths, vec!["/README.md", "/secret.txt"]);
    }

    #[test]
    fn public_audience_read_model_omits_private_files() {
        let repo = read_model_repo();
        let files = projected_files_for_audience(&repo, PUBLIC_AUDIENCE);
        let paths = files
            .into_iter()
            .map(|file| file.path.as_str().to_string())
            .collect::<Vec<_>>();

        assert_eq!(paths, vec!["/README.md"]);
    }

    fn read_model_repo() -> StoredRepository {
        let readme = crate::domain::store::SourceBlob {
            object_key: "objects/readme".to_string(),
            sha256: "readme-sha".to_string(),
            git_oid: "1111111111111111111111111111111111111111".to_string(),
            git_file_mode: crate::domain::store::DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 10,
            line_count: 1,
        };
        let secret = crate::domain::store::SourceBlob {
            object_key: "objects/secret".to_string(),
            sha256: "secret-sha".to_string(),
            git_oid: "2222222222222222222222222222222222222222".to_string(),
            git_file_mode: crate::domain::store::DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 20,
            line_count: 1,
        };

        let mut policy = Policy::new(Visibility::Private);
        policy
            .add_rule(crate::domain::policy::VisibilityRule::public(
                ScopePath::parse("/README.md").unwrap(),
            ))
            .unwrap();

        StoredRepository {
            record: RepoRecord {
                id: "owner/repo".to_string(),
                owner_handle: "owner".to_string(),
                name: "repo".to_string(),
                owner_user_id: "user-owner".to_string(),
                publication_state: RepoPublicationState::Published,
                default_visibility: Visibility::Private,
                change_version: 7,
            },
            settings: RepoSettings::default(),
            first_push_token: None,
            git_push_token: None,
            git_clone_tokens: Vec::new(),
            pending_import: None,
            policy,
            graph: SourceGraph {
                repo_id: "owner/repo".to_string(),
                commits: vec![LogicalCommit {
                    id: "c1".to_string(),
                    parent_ids: Vec::new(),
                    author_id: "user-owner".to_string(),
                    author_visibility: AuthorVisibility::Visible,
                    message: "Initial".to_string(),
                    changes: vec![
                        FileChange {
                            path: ScopePath::parse("/README.md").unwrap(),
                            old_content: None,
                            new_content: Some(readme),
                            visibility: Visibility::Public,
                        },
                        FileChange {
                            path: ScopePath::parse("/secret.txt").unwrap(),
                            old_content: None,
                            new_content: Some(secret),
                            visibility: Visibility::Private,
                        },
                    ],
                }],
            },
            visibility_events: Vec::new(),
            git_snapshot: None,
            staged_update: None,
            members: Vec::new(),
            invitations: Vec::new(),
        }
    }
}
