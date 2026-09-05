use super::{
    RepositoryStore, acquire_aggregate_lock, begin_metadata_read_snapshot, entities,
    repository_from_model,
};
use crate::error::PostgresError;
use scope_domain::{
    history::{HistoryEntry, HistoryView, history_view_from_projection},
    projection::{Projection, ProjectionViewKey, project_graph},
    repo_control::is_repo_control_path,
    repository::{Repository, RepositoryIncarnation},
};
use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement, TransactionTrait};
use std::collections::BTreeSet;

pub struct RepositoryHistoryQuery<'a> {
    pub incarnation: &'a RepositoryIncarnation,
    pub version: u64,
    pub audience: ProjectionViewKey,
    pub before_source_id: Option<&'a str>,
    pub entry_source_id: Option<&'a str>,
    pub limit: u64,
}

pub struct RepositoryHistoryPage {
    pub view: HistoryView,
    pub has_more: bool,
    pub available: bool,
}

pub(super) struct HistoryViewMetadata {
    pub generation: String,
    pub available: bool,
    pub visible_files: bool,
    pub head_oid: Option<String>,
}

pub(super) async fn history_view_metadata<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    version: u64,
    audience: ProjectionViewKey,
) -> Result<Option<HistoryViewMetadata>, PostgresError> {
    let row = conn.query_one(Statement::from_sql_and_values(DatabaseBackend::Postgres,
        "SELECT generation, available, visible_files, head_oid FROM scope_repository_history_views WHERE repo_id=$1 AND repo_version=$2 AND audience=$3 AND identity_version=$4",
        [repo_id.into(), entities::u64_to_i64(version, "repository version")?.into(), audience.as_str().into(), scope_git::PROJECTION_IDENTITY_VERSION.into()],
    )).await.map_err(PostgresError::internal)?;
    row.map(|row| {
        Ok(HistoryViewMetadata {
            generation: row
                .try_get("", "generation")
                .map_err(PostgresError::internal)?,
            available: row
                .try_get("", "available")
                .map_err(PostgresError::internal)?,
            visible_files: row
                .try_get("", "visible_files")
                .map_err(PostgresError::internal)?,
            head_oid: row
                .try_get("", "head_oid")
                .map_err(PostgresError::internal)?,
        })
    })
    .transpose()
}

/// Rebuilds the disposable history representation at one authoritative repository frontier.
/// Callers hold the repository guard; the view and its entries become visible atomically.
pub(super) async fn save_repository_history_views<C: ConnectionTrait>(
    conn: &C,
    repo: &Repository,
) -> Result<(), PostgresError> {
    for audience in [ProjectionViewKey::Private, ProjectionViewKey::Public] {
        let projection = project_graph(&repo.graph, &repo.visibility_change_sets, audience);
        let head_oid =
            scope_git::projection_head_oid(&projection).map_err(PostgresError::internal)?;
        save_repository_history_view(conn, repo, projection, head_oid).await?;
    }
    Ok(())
}

pub(super) async fn save_repository_history_view<C: ConnectionTrait>(
    conn: &C,
    repo: &Repository,
    projection: Projection,
    head_oid: Option<String>,
) -> Result<(), PostgresError> {
    let audience = projection.view_key;
    let available = !projection.preserves_git_commits();
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "DELETE FROM scope_repository_history_views WHERE repo_id=$1 AND audience=$2",
        [repo.record.id.clone().into(), audience.as_str().into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    let mut tree = BTreeSet::new();
    for change in projection.commits.iter().flat_map(|commit| &commit.changes) {
        if change.new_content.is_some() {
            tree.insert(&change.path);
        } else {
            tree.remove(&change.path);
        }
    }
    let visible_files = tree.into_iter().any(|path| !is_repo_control_path(path));
    let view = history_view_from_projection(projection, &repo.graph, &repo.visibility_change_sets);
    conn.execute(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "INSERT INTO scope_repository_history_views (repo_id,audience,repo_version,generation,available,visible_files,head_oid,identity_version) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            [repo.record.id.clone().into(), audience.as_str().into(), entities::u64_to_i64(repo.record.change_version,"repository version")?.into(), view.generation.into(), available.into(), visible_files.into(), head_oid.into(), scope_git::PROJECTION_IDENTITY_VERSION.into()],
        )).await.map_err(PostgresError::internal)?;
    for batch in view
        .entries
        .iter()
        .rev()
        .enumerate()
        .collect::<Vec<_>>()
        .chunks(500)
    {
        let mut values = Vec::with_capacity(batch.len() * 5);
        let rows = batch
            .iter()
            .enumerate()
            .map(|(index, (position, entry))| {
                let offset = index * 5;
                values.extend([
                    repo.record.id.clone().into(),
                    audience.as_str().into(),
                    (*position as i64).into(),
                    entry.source_id.clone().into(),
                    serde_json::to_value(entry)
                        .map_err(PostgresError::internal)?
                        .into(),
                ]);
                Ok(format!(
                    "(${},${},${},${},${})",
                    offset + 1,
                    offset + 2,
                    offset + 3,
                    offset + 4,
                    offset + 5
                ))
            })
            .collect::<Result<Vec<_>, PostgresError>>()?
            .join(",");
        conn.execute(Statement::from_sql_and_values(DatabaseBackend::Postgres,
                format!("INSERT INTO scope_repository_history_entries (repo_id,audience,position,source_id,payload) VALUES {rows}"), values,
            )).await.map_err(PostgresError::internal)?;
    }
    Ok(())
}

impl RepositoryStore {
    /// Concurrent cache misses recheck after acquiring the repository guard, so one build
    /// supplies every reader. A mutation invalidates the cache by advancing change_version.
    pub async fn ensure_history_view(
        &self,
        incarnation: &RepositoryIncarnation,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", incarnation.repository_id()).await?;
        let row = entities::repository::Entity::find_by_id(incarnation.repository_id())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        if row.incarnation_id != incarnation.incarnation_id() {
            return Err(PostgresError::conflict(
                "repository was recreated; retry the read",
            ));
        }
        let version = entities::i64_to_u64(row.change_version, "repository version")?;
        if history_view_metadata(&tx, &row.id, version, ProjectionViewKey::Private)
            .await?
            .is_none()
            || history_view_metadata(&tx, &row.id, version, ProjectionViewKey::Public)
                .await?
                .is_none()
        {
            let repo = repository_from_model(&tx, row).await?;
            save_repository_history_views(&tx, &repo).await?;
        }
        tx.commit().await.map_err(PostgresError::internal)
    }

    pub async fn repository_history_page(
        &self,
        query: RepositoryHistoryQuery<'_>,
    ) -> Result<RepositoryHistoryPage, PostgresError> {
        let RepositoryHistoryQuery {
            incarnation,
            version,
            audience,
            before_source_id,
            entry_source_id,
            limit,
        } = query;
        for _ in 0..2 {
            let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
            let current =
                super::repository_access::repository_access(&tx, incarnation.repository_id(), None)
                    .await?
                    .ok_or_else(|| PostgresError::not_found("repo not found"))?;
            if current.incarnation() != *incarnation || current.record.change_version != version {
                return Err(PostgresError::conflict(
                    "repository changed while reading history; retry",
                ));
            }
            let Some(metadata) =
                history_view_metadata(&tx, incarnation.repository_id(), version, audience).await?
            else {
                tx.commit().await.map_err(PostgresError::internal)?;
                self.ensure_history_view(incarnation).await?;
                continue;
            };
            let boundary = match before_source_id {
            Some(source_id) => Some(tx.query_one(Statement::from_sql_and_values(DatabaseBackend::Postgres,
                "SELECT position FROM scope_repository_history_entries WHERE repo_id=$1 AND audience=$2 AND source_id=$3",
                [incarnation.repository_id().into(), audience.as_str().into(), source_id.into()],
            )).await.map_err(PostgresError::internal)?.ok_or_else(|| PostgresError::invalid_input("history cursor boundary is no longer available"))?
                .try_get::<i64>("", "position").map_err(PostgresError::internal)?),
            None => None,
        };
            let limit = limit.clamp(1, 50) as i64;
            // Separate predicates retain index bounds even after PostgreSQL chooses a generic plan.
            let mut values = vec![incarnation.repository_id().into(), audience.as_str().into()];
            let sql = if let Some(source_id) = entry_source_id {
                values.push(source_id.into());
                "SELECT payload FROM scope_repository_history_entries WHERE repo_id=$1 AND audience=$2 AND source_id=$3 LIMIT 1"
            } else if let Some(position) = boundary {
                values.extend([position.into(), (limit + 1).into()]);
                "SELECT payload FROM scope_repository_history_entries WHERE repo_id=$1 AND audience=$2 AND position<$3 ORDER BY position DESC LIMIT $4"
            } else {
                values.push((limit + 1).into());
                "SELECT payload FROM scope_repository_history_entries WHERE repo_id=$1 AND audience=$2 ORDER BY position DESC LIMIT $3"
            };
            let rows = tx
                .query_all(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    sql,
                    values,
                ))
                .await
                .map_err(PostgresError::internal)?;
            let mut entries = rows
                .into_iter()
                .map(|row| {
                    serde_json::from_value::<HistoryEntry>(
                        row.try_get("", "payload")
                            .map_err(PostgresError::internal)?,
                    )
                    .map_err(PostgresError::internal)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let has_more = entries.len() > limit as usize;
            entries.truncate(limit as usize);
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(RepositoryHistoryPage {
                view: HistoryView {
                    repo_id: incarnation.repository_id().to_string(),
                    view_key: audience.as_str().to_string(),
                    generation: metadata.generation,
                    entries,
                },
                has_more,
                available: metadata.available,
            });
        }
        Err(PostgresError::conflict(
            "repository changed while reading history; retry",
        ))
    }
}

#[cfg(test)]
mod tests;
