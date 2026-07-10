use super::{
    MetadataStore, begin_metadata_read_snapshot, entities,
    projection_encoding::ProjectionAudience,
    projection_read_models::{
        load_live_projection_file_count_for_audience, load_live_projection_files_for_audience,
    },
    repository_from_model,
};
use crate::{
    domain::{
        policy::{Policy, Principal, PrincipalKind, ScopePath, Visibility},
        projection_views::{
            ProjectionViewFile, ProjectionViewFileContent, has_visible_projected_history,
            projected_file_content as domain_projected_file_content,
            projected_files as domain_projected_files,
        },
        store::{
            RepoPublicationState, RepositoryAccess, RepositoryActor, RepositoryMemberPermissions,
            StoredRepository, repo_id,
        },
    },
    error::ApiError,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, prelude::Json,
};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoSummaryRead {
    pub id: String,
    pub owner_handle: String,
    pub name: String,
    pub lifecycle_state: RepoPublicationState,
    pub default_visibility: Visibility,
    pub change_version: u64,
    pub access: RepositoryAccess,
}

#[derive(Clone, Debug, FromQueryResult)]
struct RepoReadRow {
    id: String,
    owner_handle: String,
    name: String,
    owner_user_id: String,
    publication_state: String,
    default_visibility: String,
    change_version: i64,
    policy: Json,
}

impl MetadataStore {
    pub async fn repo_summaries_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<RepoSummaryRead>, ApiError> {
        let user_id = user_id.to_string();
        let db = Arc::clone(&self.db);
        let tx = begin_metadata_read_snapshot(db.as_ref()).await?;
        let summaries = repo_summaries_for_user_tx(&tx, &user_id).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(summaries)
    }

    pub async fn repo_summary(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<RepoSummaryRead>, ApiError> {
        let owner = owner.to_string();
        let name = name.to_string();
        let viewer_user_id = viewer_user_id.map(str::to_string);
        let db = Arc::clone(&self.db);
        let tx = begin_metadata_read_snapshot(db.as_ref()).await?;
        let summary = repo_summary_tx(&tx, &owner, &name, viewer_user_id.as_deref()).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(summary)
    }

    pub async fn repo_live_files(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<Vec<ProjectionViewFile>>, ApiError> {
        let owner = owner.to_string();
        let name = name.to_string();
        let viewer_user_id = viewer_user_id.map(str::to_string);
        let db = Arc::clone(&self.db);
        let tx = begin_metadata_read_snapshot(db.as_ref()).await?;
        let files = repo_live_files_tx(&tx, &owner, &name, viewer_user_id.as_deref()).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(files)
    }

    pub async fn repo_live_file_content(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
        path: &ScopePath,
    ) -> Result<Option<ProjectionViewFileContent>, ApiError> {
        let owner = owner.to_string();
        let name = name.to_string();
        let viewer_user_id = viewer_user_id.map(str::to_string);
        let path = path.clone();
        let db = Arc::clone(&self.db);
        let tx = begin_metadata_read_snapshot(db.as_ref()).await?;
        let content =
            repo_live_file_content_tx(&tx, &owner, &name, viewer_user_id.as_deref(), &path).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(content)
    }
}

async fn repo_summaries_for_user_tx<C>(
    conn: &C,
    user_id: &str,
) -> Result<Vec<RepoSummaryRead>, ApiError>
where
    C: ConnectionTrait,
{
    let owner_rows = repo_read_rows_for_owner(conn, user_id).await?;
    let member_rows = entities::repository_member::Entity::find()
        .filter(entities::repository_member::Column::UserId.eq(user_id.to_string()))
        .order_by_asc(entities::repository_member::Column::RepoId)
        .all(conn)
        .await
        .map_err(ApiError::internal)?;
    let mut member_permissions = BTreeMap::new();
    for member in member_rows {
        let repo_id = member.repo_id.clone();
        member_permissions.insert(repo_id, member.try_into_domain()?.permissions);
    }
    let member_repo_ids = member_permissions.keys().cloned().collect::<Vec<_>>();
    let member_repo_rows = repo_read_rows_by_ids(conn, member_repo_ids).await?;

    let mut rows = owner_rows
        .into_iter()
        .map(|row| (row.id.clone(), (row, None)))
        .collect::<BTreeMap<_, _>>();
    for row in member_repo_rows {
        if row.owner_user_id == user_id {
            continue;
        }
        let permissions = member_permissions.get(&row.id).copied();
        rows.entry(row.id.clone()).or_insert((row, permissions));
    }

    let mut summaries = Vec::new();
    for (row, permissions) in rows.into_values() {
        let access = access_for_row(&row, Some(user_id), permissions)?;
        if let Some(summary) = summary_for_user_list_row(row, access)? {
            summaries.push(summary);
        }
    }
    summaries.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(summaries)
}

async fn repo_summary_tx<C>(
    conn: &C,
    owner: &str,
    name: &str,
    viewer_user_id: Option<&str>,
) -> Result<Option<RepoSummaryRead>, ApiError>
where
    C: ConnectionTrait,
{
    let Some(row) = repo_read_row_by_owner_name(conn, owner, name).await? else {
        return Ok(None);
    };
    let permissions = member_permissions_for_viewer(conn, &row, viewer_user_id).await?;
    let access = access_for_row(&row, viewer_user_id, permissions)?;
    summary_for_viewer_row(conn, row, viewer_user_id, access).await
}

async fn repo_live_files_tx<C>(
    conn: &C,
    owner: &str,
    name: &str,
    viewer_user_id: Option<&str>,
) -> Result<Option<Vec<ProjectionViewFile>>, ApiError>
where
    C: ConnectionTrait,
{
    let Some(row) = repo_read_row_by_owner_name(conn, owner, name).await? else {
        return Ok(None);
    };
    let permissions = member_permissions_for_viewer(conn, &row, viewer_user_id).await?;
    let access = access_for_row(&row, viewer_user_id, permissions)?;
    let audience = live_projection_audience(access);

    if let Some(files) =
        load_live_projection_files_for_audience(conn, &row.id, row.change_version()?, audience)
            .await?
    {
        let visible_projection = if !files.is_empty() {
            true
        } else if needs_public_projection_visibility(&row, viewer_user_id, access) {
            let repo = hydrate_repo_from_row_id(conn, &row.id).await?;
            let principal = principal_for_access(viewer_user_id, access);
            has_visible_projected_history(&repo, &principal)
        } else {
            false
        };
        return if row_is_readable_with_visible_projection(&row, access, visible_projection)? {
            Ok(Some(files))
        } else {
            Ok(None)
        };
    }

    let repo = hydrate_repo_from_row_id(conn, &row.id).await?;
    let principal = principal_for_access(viewer_user_id, access);
    let visible_projection = repo.record.publication_state == RepoPublicationState::Published
        && has_visible_projected_history(&repo, &principal);
    if !row_is_readable_with_visible_projection(&row, access, visible_projection)? {
        return Ok(None);
    }
    Ok(Some(domain_projected_files(&repo, &principal)))
}

async fn repo_live_file_content_tx<C>(
    conn: &C,
    owner: &str,
    name: &str,
    viewer_user_id: Option<&str>,
    path: &ScopePath,
) -> Result<Option<ProjectionViewFileContent>, ApiError>
where
    C: ConnectionTrait,
{
    let Some(row) = repo_read_row_by_owner_name(conn, owner, name).await? else {
        return Ok(None);
    };
    let permissions = member_permissions_for_viewer(conn, &row, viewer_user_id).await?;
    let access = access_for_row(&row, viewer_user_id, permissions)?;
    if !row_is_readable_for_viewer(conn, &row, viewer_user_id, access).await? {
        return Ok(None);
    }
    let repo = hydrate_repo_from_row_id(conn, &row.id).await?;
    let principal = principal_for_access(viewer_user_id, access);
    Ok(domain_projected_file_content(&repo, &principal, path))
}

async fn repo_read_row_by_owner_name<C>(
    conn: &C,
    owner: &str,
    name: &str,
) -> Result<Option<RepoReadRow>, ApiError>
where
    C: ConnectionTrait,
{
    let id = repo_id(owner, name);
    repo_read_query()
        .filter(entities::repository::Column::Id.eq(id))
        .into_model::<RepoReadRow>()
        .one(conn)
        .await
        .map_err(ApiError::internal)
}

async fn repo_read_rows_for_owner<C>(conn: &C, user_id: &str) -> Result<Vec<RepoReadRow>, ApiError>
where
    C: ConnectionTrait,
{
    repo_read_query()
        .filter(entities::repository::Column::OwnerUserId.eq(user_id.to_string()))
        .order_by_asc(entities::repository::Column::Id)
        .into_model::<RepoReadRow>()
        .all(conn)
        .await
        .map_err(ApiError::internal)
}

async fn repo_read_rows_by_ids<C>(
    conn: &C,
    repo_ids: Vec<String>,
) -> Result<Vec<RepoReadRow>, ApiError>
where
    C: ConnectionTrait,
{
    if repo_ids.is_empty() {
        return Ok(Vec::new());
    }
    repo_read_query()
        .filter(entities::repository::Column::Id.is_in(repo_ids))
        .order_by_asc(entities::repository::Column::Id)
        .into_model::<RepoReadRow>()
        .all(conn)
        .await
        .map_err(ApiError::internal)
}

fn repo_read_query() -> sea_orm::Select<entities::repository::Entity> {
    entities::repository::Entity::find()
        .select_only()
        .column(entities::repository::Column::Id)
        .column(entities::repository::Column::OwnerHandle)
        .column(entities::repository::Column::Name)
        .column(entities::repository::Column::OwnerUserId)
        .column(entities::repository::Column::PublicationState)
        .column(entities::repository::Column::DefaultVisibility)
        .column(entities::repository::Column::ChangeVersion)
        .column(entities::repository::Column::Policy)
}

async fn member_permissions_for_viewer<C>(
    conn: &C,
    row: &RepoReadRow,
    viewer_user_id: Option<&str>,
) -> Result<Option<RepositoryMemberPermissions>, ApiError>
where
    C: ConnectionTrait,
{
    let Some(user_id) = viewer_user_id else {
        return Ok(None);
    };
    if user_id == row.owner_user_id {
        return Ok(None);
    }
    let Some(member) = entities::repository_member::Entity::find()
        .filter(entities::repository_member::Column::RepoId.eq(row.id.clone()))
        .filter(entities::repository_member::Column::UserId.eq(user_id.to_string()))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
    else {
        return Ok(None);
    };
    Ok(Some(member.try_into_domain()?.permissions))
}

async fn hydrate_repo_from_row_id<C>(conn: &C, repo_id: &str) -> Result<StoredRepository, ApiError>
where
    C: ConnectionTrait,
{
    let row = entities::repository::Entity::find_by_id(repo_id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal_message("repository row disappeared while reading"))?;
    repository_from_model(conn, row).await
}

async fn summary_for_viewer_row<C>(
    conn: &C,
    row: RepoReadRow,
    viewer_user_id: Option<&str>,
    access: RepositoryAccess,
) -> Result<Option<RepoSummaryRead>, ApiError>
where
    C: ConnectionTrait,
{
    let visible_projection = if needs_public_projection_visibility(&row, viewer_user_id, access) {
        match load_live_projection_file_count_for_audience(
            conn,
            &row.id,
            row.change_version()?,
            ProjectionAudience::Public,
        )
        .await?
        {
            Some(count) if count > 0 => true,
            Some(_) | None => {
                let repo = hydrate_repo_from_row_id(conn, &row.id).await?;
                let principal = principal_for_access(viewer_user_id, access);
                has_visible_projected_history(&repo, &principal)
            }
        }
    } else {
        false
    };

    if !row_is_readable_with_visible_projection(&row, access, visible_projection)? {
        return Ok(None);
    }
    Ok(Some(summary_from_row(row, access)?))
}

async fn row_is_readable_for_viewer<C>(
    conn: &C,
    row: &RepoReadRow,
    viewer_user_id: Option<&str>,
    access: RepositoryAccess,
) -> Result<bool, ApiError>
where
    C: ConnectionTrait,
{
    if !needs_public_projection_visibility(row, viewer_user_id, access) {
        return row_is_readable_with_visible_projection(row, access, false);
    }
    let visible_projection = match load_live_projection_file_count_for_audience(
        conn,
        &row.id,
        row.change_version()?,
        ProjectionAudience::Public,
    )
    .await?
    {
        Some(count) if count > 0 => true,
        Some(_) | None => {
            let repo = hydrate_repo_from_row_id(conn, &row.id).await?;
            let principal = principal_for_access(viewer_user_id, access);
            has_visible_projected_history(&repo, &principal)
        }
    };
    row_is_readable_with_visible_projection(row, access, visible_projection)
}

fn summary_for_user_list_row(
    row: RepoReadRow,
    access: RepositoryAccess,
) -> Result<Option<RepoSummaryRead>, ApiError> {
    if access.actor == RepositoryActor::Public {
        return Ok(None);
    }
    let publication_state = row.publication_state()?;
    let lifecycle_allows_read = publication_state == RepoPublicationState::Published
        || access.actor == RepositoryActor::Owner;
    let policy = row.policy()?;
    if !lifecycle_allows_read || !policy.can_read(&ScopePath::root(), access.can_read_private_files)
    {
        return Ok(None);
    }
    Ok(Some(summary_from_row(row, access)?))
}

fn summary_from_row(
    row: RepoReadRow,
    access: RepositoryAccess,
) -> Result<RepoSummaryRead, ApiError> {
    let lifecycle_state = row.publication_state()?;
    let default_visibility = row.default_visibility()?;
    let change_version = repo_change_version_for_access(row.change_version()?, access);
    Ok(RepoSummaryRead {
        id: row.id,
        owner_handle: row.owner_handle,
        name: row.name,
        lifecycle_state,
        default_visibility,
        change_version,
        access,
    })
}

fn access_for_row(
    row: &RepoReadRow,
    viewer_user_id: Option<&str>,
    member_permissions: Option<RepositoryMemberPermissions>,
) -> Result<RepositoryAccess, ApiError> {
    let Some(user_id) = viewer_user_id else {
        return Ok(RepositoryAccess::public());
    };
    let publication_state = row.publication_state()?;
    Ok(access_for_user_id(
        user_id,
        &row.owner_user_id,
        publication_state,
        member_permissions,
    ))
}

fn access_for_user_id(
    user_id: &str,
    owner_user_id: &str,
    publication_state: RepoPublicationState,
    member_permissions: Option<RepositoryMemberPermissions>,
) -> RepositoryAccess {
    let published = publication_state == RepoPublicationState::Published;
    if user_id == owner_user_id {
        return RepositoryAccess {
            actor: RepositoryActor::Owner,
            can_read_private_files: true,
            can_push: published,
            can_change_file_visibility: true,
            can_apply_changes: true,
            can_manage_members: published,
            can_delete_repo: true,
        };
    }
    let Some(permissions) = member_permissions else {
        return RepositoryAccess::public();
    };
    RepositoryAccess {
        actor: RepositoryActor::Member,
        can_read_private_files: published,
        can_push: published && permissions.can_push,
        can_change_file_visibility: published && permissions.can_change_file_visibility,
        can_apply_changes: published && permissions.can_apply_changes,
        can_manage_members: false,
        can_delete_repo: false,
    }
}

fn row_is_readable_with_visible_projection(
    row: &RepoReadRow,
    access: RepositoryAccess,
    visible_projection: bool,
) -> Result<bool, ApiError> {
    let publication_state = row.publication_state()?;
    let policy = row.policy()?;
    Ok(readable_from_facts(
        publication_state,
        &policy,
        access.actor == RepositoryActor::Public,
        access,
        visible_projection,
    ))
}

fn needs_public_projection_visibility(
    row: &RepoReadRow,
    _viewer_user_id: Option<&str>,
    access: RepositoryAccess,
) -> bool {
    if access.actor != RepositoryActor::Public {
        return false;
    }
    if row.publication_state().ok() != Some(RepoPublicationState::Published) {
        return false;
    }
    let Ok(policy) = row.policy() else {
        return true;
    };
    !policy.can_read(&ScopePath::root(), false)
}

fn readable_from_facts(
    publication_state: RepoPublicationState,
    policy: &Policy,
    principal_is_public: bool,
    access: RepositoryAccess,
    visible_projection: bool,
) -> bool {
    let root = ScopePath::root();
    match access.actor {
        RepositoryActor::Owner => policy.can_read(&root, true),
        RepositoryActor::Member => {
            publication_state == RepoPublicationState::Published
                && policy.can_read(&root, access.can_read_private_files)
        }
        RepositoryActor::Public => {
            publication_state == RepoPublicationState::Published
                && ((principal_is_public && policy.can_read(&root, false)) || visible_projection)
        }
    }
}

fn live_projection_audience(access: RepositoryAccess) -> ProjectionAudience {
    if access.actor != RepositoryActor::Public && access.can_read_private_files {
        ProjectionAudience::Private
    } else {
        ProjectionAudience::Public
    }
}

fn principal_for_viewer(viewer_user_id: Option<&str>) -> Principal {
    match viewer_user_id {
        Some(user_id) => Principal {
            id: user_id.to_string(),
            kind: PrincipalKind::User,
        },
        None => Principal::public(),
    }
}

fn principal_for_access(viewer_user_id: Option<&str>, access: RepositoryAccess) -> Principal {
    if access.actor == RepositoryActor::Public {
        return Principal::public();
    }
    principal_for_viewer(viewer_user_id)
}

fn repo_change_version_for_access(change_version: u64, access: RepositoryAccess) -> u64 {
    if access.actor != RepositoryActor::Public {
        change_version
    } else {
        0
    }
}

fn decode_enum<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, ApiError> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).map_err(ApiError::internal)
}

impl RepoReadRow {
    fn publication_state(&self) -> Result<RepoPublicationState, ApiError> {
        decode_enum(&self.publication_state)
    }

    fn default_visibility(&self) -> Result<Visibility, ApiError> {
        decode_enum(&self.default_visibility)
    }

    fn policy(&self) -> Result<Policy, ApiError> {
        serde_json::from_value(self.policy.clone()).map_err(ApiError::internal)
    }

    fn change_version(&self) -> Result<u64, ApiError> {
        u64::try_from(self.change_version)
            .map_err(|_| ApiError::internal_message("repository change version cannot be negative"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DbBackend, QueryTrait};

    #[test]
    fn repo_read_query_skips_aggregate_json_columns() {
        let sql = repo_read_query().build(DbBackend::Postgres).to_string();

        assert!(
            !sql.contains("\"scope_repositories\".\"graph\""),
            "narrow repo reads must not select graph JSON: {sql}"
        );
        assert!(
            !sql.contains("\"scope_repositories\".\"visibility_events\""),
            "narrow repo reads must not select visibility event JSON: {sql}"
        );
    }
}
