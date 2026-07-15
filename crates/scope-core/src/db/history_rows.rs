use super::entities;
use super::object_references::insert_object_reference;
use crate::domain::store::SourceBlob;
use crate::domain::{
    policy::ScopePath,
    projection::{FileChange, LogicalCommit, SourceGraph, VisibilityEvent},
};
use crate::error::ApiError;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect,
};
use std::collections::BTreeMap;

pub struct RepositoryHistory {
    pub graph: SourceGraph,
    pub visibility_events: Vec<VisibilityEvent>,
    pub live_files: BTreeMap<ScopePath, SourceBlob>,
}

pub async fn insert_repository_history<C>(
    conn: &C,
    graph: &SourceGraph,
    visibility_events: &[VisibilityEvent],
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    insert_commits(conn, &graph.repo_id, 0, &graph.commits).await?;
    insert_visibility_events(conn, &graph.repo_id, 0, visibility_events).await
}

pub struct RepositoryHistoryDelta<'a> {
    pub before_graph: &'a SourceGraph,
    pub after_graph: &'a SourceGraph,
    pub before_events: &'a [VisibilityEvent],
    pub after_events: &'a [VisibilityEvent],
    pub before_live_files: &'a BTreeMap<ScopePath, SourceBlob>,
    pub after_live_files: &'a BTreeMap<ScopePath, SourceBlob>,
    pub history_rewritten: bool,
}

pub async fn save_repository_history_delta<C>(
    conn: &C,
    delta: RepositoryHistoryDelta<'_>,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let RepositoryHistoryDelta {
        before_graph,
        after_graph,
        before_events,
        after_events,
        before_live_files,
        after_live_files,
        history_rewritten,
    } = delta;
    let commits_are_append_only = after_graph.commits.len() >= before_graph.commits.len()
        && after_graph.commits[..before_graph.commits.len()] == before_graph.commits[..];
    let events_are_append_only = after_events.len() >= before_events.len()
        && after_events[..before_events.len()] == before_events[..];
    let commits_rewritten = history_rewritten || !commits_are_append_only;
    let events_rewritten = history_rewritten || !events_are_append_only;

    if commits_rewritten {
        delete_owned_history_references(conn, "file_change", &after_graph.repo_id).await?;
        replace_commits(conn, after_graph).await?;
    } else if after_graph.commits.len() > before_graph.commits.len() {
        insert_commits(
            conn,
            &after_graph.repo_id,
            before_graph.commits.len(),
            &after_graph.commits[before_graph.commits.len()..],
        )
        .await?;
    }

    if events_rewritten {
        delete_owned_history_references(conn, "visibility_event", &after_graph.repo_id).await?;
        entities::visibility_event::Entity::delete_many()
            .filter(entities::visibility_event::Column::RepoId.eq(after_graph.repo_id.clone()))
            .exec(conn)
            .await
            .map_err(ApiError::internal)?;
        insert_visibility_events(conn, &after_graph.repo_id, 0, after_events).await?;
    } else if after_events.len() > before_events.len() {
        insert_visibility_events(
            conn,
            &after_graph.repo_id,
            before_events.len(),
            &after_events[before_events.len()..],
        )
        .await?;
    }
    if commits_rewritten || after_graph.commits.len() != before_graph.commits.len() + 1 {
        if before_live_files != after_live_files {
            replace_live_files(conn, &after_graph.repo_id, after_live_files).await?;
        }
    } else if let Some(commit) = after_graph.commits.last() {
        for change in &commit.changes {
            save_live_file(
                conn,
                &after_graph.repo_id,
                &change.path,
                after_live_files.get(&change.path),
            )
            .await?;
        }
    }
    Ok(())
}

pub async fn load_repository_histories<C>(
    conn: &C,
    repo_ids: &[String],
) -> Result<BTreeMap<String, RepositoryHistory>, ApiError>
where
    C: ConnectionTrait,
{
    let mut histories = repo_ids
        .iter()
        .map(|repo_id| {
            (
                repo_id.clone(),
                RepositoryHistory {
                    graph: SourceGraph {
                        repo_id: repo_id.clone(),
                        commits: Vec::new(),
                    },
                    visibility_events: Vec::new(),
                    live_files: BTreeMap::new(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    if repo_ids.is_empty() {
        return Ok(histories);
    }

    let commits = entities::logical_commit::Entity::find()
        .filter(entities::logical_commit::Column::RepoId.is_in(repo_ids.to_vec()))
        .order_by_asc(entities::logical_commit::Column::RepoId)
        .order_by_asc(entities::logical_commit::Column::Ordinal)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    let commit_ids = commits.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
    let mut changes_by_commit = BTreeMap::<(String, String), Vec<FileChange>>::new();
    if !commit_ids.is_empty() {
        for row in entities::file_change::Entity::find()
            .filter(entities::file_change::Column::RepoId.is_in(repo_ids.to_vec()))
            .filter(entities::file_change::Column::CommitId.is_in(commit_ids))
            .order_by_asc(entities::file_change::Column::RepoId)
            .order_by_asc(entities::file_change::Column::CommitId)
            .order_by_asc(entities::file_change::Column::Ordinal)
            .all(conn)
            .await
            .map_err(ApiError::internal)?
        {
            changes_by_commit
                .entry((row.repo_id.clone(), row.commit_id.clone()))
                .or_default()
                .push(FileChange {
                    path: ScopePath::parse(row.path).map_err(ApiError::internal)?,
                    old_content: decode_optional(row.old_content)?,
                    new_content: decode_optional(row.new_content)?,
                    visibility: decode_enum(row.visibility)?,
                });
        }
    }
    for row in commits {
        if let Some(history) = histories.get_mut(&row.repo_id) {
            history.graph.commits.push(LogicalCommit {
                id: row.id.clone(),
                parent_ids: serde_json::from_value(row.parent_ids).map_err(ApiError::internal)?,
                author_id: row.author_id,
                author_visibility: decode_enum(row.author_visibility)?,
                message: row.message,
                changes: changes_by_commit
                    .remove(&(row.repo_id.clone(), row.id.clone()))
                    .unwrap_or_default(),
            });
        }
    }

    for row in entities::visibility_event::Entity::find()
        .filter(entities::visibility_event::Column::RepoId.is_in(repo_ids.to_vec()))
        .order_by_asc(entities::visibility_event::Column::RepoId)
        .order_by_asc(entities::visibility_event::Column::Ordinal)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
    {
        if let Some(history) = histories.get_mut(&row.repo_id) {
            history.visibility_events.push(VisibilityEvent {
                id: row.id,
                after_commit_id: row.after_commit_id,
                source_commit_id: row.source_commit_id,
                author_id: row.author_id,
                path: ScopePath::parse(row.path).map_err(ApiError::internal)?,
                old_visibility: decode_enum(row.old_visibility)?,
                new_visibility: decode_enum(row.new_visibility)?,
                current_content: decode_optional(row.current_content)?,
            });
        }
    }
    for row in entities::live_file::Entity::find()
        .filter(entities::live_file::Column::RepoId.is_in(repo_ids.to_vec()))
        .order_by_asc(entities::live_file::Column::RepoId)
        .order_by_asc(entities::live_file::Column::Path)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
    {
        if let Some(history) = histories.get_mut(&row.repo_id) {
            history.live_files.insert(
                ScopePath::parse(row.path).map_err(ApiError::internal)?,
                serde_json::from_value(row.content).map_err(ApiError::internal)?,
            );
        }
    }
    Ok(histories)
}

pub async fn insert_repository_live_files<C>(
    conn: &C,
    repo_id: &str,
    live_files: &BTreeMap<ScopePath, SourceBlob>,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    for (path, content) in live_files {
        save_live_file(conn, repo_id, path, Some(content)).await?;
    }
    Ok(())
}

async fn replace_live_files<C>(
    conn: &C,
    repo_id: &str,
    live_files: &BTreeMap<ScopePath, SourceBlob>,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::live_file::Entity::delete_many()
        .filter(entities::live_file::Column::RepoId.eq(repo_id.to_string()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    insert_repository_live_files(conn, repo_id, live_files).await
}

pub(super) async fn save_live_file<C>(
    conn: &C,
    repo_id: &str,
    path: &ScopePath,
    content: Option<&SourceBlob>,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::live_file::Entity::delete_by_id((repo_id.to_string(), path.as_str().to_string()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    if let Some(content) = content {
        entities::live_file::Model {
            repo_id: repo_id.to_string(),
            path: path.as_str().to_string(),
            content: serde_json::to_value(content).map_err(ApiError::internal)?,
        }
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;
    }
    Ok(())
}

async fn replace_commits<C>(conn: &C, graph: &SourceGraph) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let commit_ids = entities::logical_commit::Entity::find()
        .select_only()
        .column(entities::logical_commit::Column::Id)
        .filter(entities::logical_commit::Column::RepoId.eq(graph.repo_id.clone()))
        .into_tuple::<String>()
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    if !commit_ids.is_empty() {
        entities::file_change::Entity::delete_many()
            .filter(entities::file_change::Column::RepoId.eq(graph.repo_id.clone()))
            .filter(entities::file_change::Column::CommitId.is_in(commit_ids))
            .exec(conn)
            .await
            .map_err(ApiError::internal)?;
    }
    entities::logical_commit::Entity::delete_many()
        .filter(entities::logical_commit::Column::RepoId.eq(graph.repo_id.clone()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    insert_commits(conn, &graph.repo_id, 0, &graph.commits).await
}

pub(super) async fn insert_commits<C>(
    conn: &C,
    repo_id: &str,
    ordinal_offset: usize,
    commits: &[LogicalCommit],
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    for (offset, commit) in commits.iter().enumerate() {
        entities::logical_commit::Model {
            id: commit.id.clone(),
            repo_id: repo_id.to_string(),
            ordinal: usize_to_i64(ordinal_offset + offset)?,
            parent_ids: serde_json::to_value(&commit.parent_ids).map_err(ApiError::internal)?,
            author_id: commit.author_id.clone(),
            author_visibility: encode_enum(&commit.author_visibility)?,
            message: commit.message.clone(),
        }
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;
        for (ordinal, change) in commit.changes.iter().enumerate() {
            entities::file_change::Model {
                repo_id: repo_id.to_string(),
                commit_id: commit.id.clone(),
                ordinal: usize_to_i64(ordinal)?,
                path: change.path.as_str().to_string(),
                old_content: encode_optional(change.old_content.as_ref())?,
                new_content: encode_optional(change.new_content.as_ref())?,
                visibility: encode_enum(&change.visibility)?,
            }
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
            for (side, content) in [
                ("old", change.old_content.as_ref()),
                ("new", change.new_content.as_ref()),
            ] {
                if let Some(content) = content
                    && !content.object_key.starts_with("git-blobs/")
                {
                    insert_object_reference(
                        conn,
                        "file_change",
                        &format!("{repo_id}:{}:{ordinal}:{side}", commit.id),
                        content,
                    )
                    .await?;
                }
            }
        }
    }
    Ok(())
}

async fn insert_visibility_events<C>(
    conn: &C,
    repo_id: &str,
    ordinal_offset: usize,
    events: &[VisibilityEvent],
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    for (offset, event) in events.iter().enumerate() {
        entities::visibility_event::Model {
            repo_id: repo_id.to_string(),
            id: event.id.clone(),
            ordinal: usize_to_i64(ordinal_offset + offset)?,
            after_commit_id: event.after_commit_id.clone(),
            source_commit_id: event.source_commit_id.clone(),
            author_id: event.author_id.clone(),
            path: event.path.as_str().to_string(),
            old_visibility: encode_enum(&event.old_visibility)?,
            new_visibility: encode_enum(&event.new_visibility)?,
            current_content: encode_optional(event.current_content.as_ref())?,
        }
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;
        if let Some(content) = event.current_content.as_ref()
            && !content.object_key.starts_with("git-blobs/")
        {
            insert_object_reference(
                conn,
                "visibility_event",
                &format!("{repo_id}:{}", event.id),
                content,
            )
            .await?;
        }
    }
    Ok(())
}

async fn delete_owned_history_references<C>(
    conn: &C,
    ref_kind: &str,
    repo_id: &str,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::object_reference::Entity::delete_many()
        .filter(entities::object_reference::Column::RefKind.eq(ref_kind.to_string()))
        .filter(entities::object_reference::Column::RefId.starts_with(format!("{repo_id}:")))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

fn encode_optional<T: serde::Serialize>(
    value: Option<&T>,
) -> Result<Option<serde_json::Value>, ApiError> {
    value
        .map(|value| serde_json::to_value(value).map_err(ApiError::internal))
        .transpose()
}

fn decode_optional<T: serde::de::DeserializeOwned>(
    value: Option<serde_json::Value>,
) -> Result<Option<T>, ApiError> {
    value
        .map(|value| serde_json::from_value(value).map_err(ApiError::internal))
        .transpose()
}

fn encode_enum<T: serde::Serialize>(value: &T) -> Result<String, ApiError> {
    serde_json::to_value(value)
        .map_err(ApiError::internal)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ApiError::internal_message("enum did not serialize to a string"))
}

fn decode_enum<T: serde::de::DeserializeOwned>(value: String) -> Result<T, ApiError> {
    serde_json::from_value(serde_json::Value::String(value)).map_err(ApiError::internal)
}

fn usize_to_i64(value: usize) -> Result<i64, ApiError> {
    i64::try_from(value).map_err(|_| ApiError::internal_message("history ordinal overflow"))
}
