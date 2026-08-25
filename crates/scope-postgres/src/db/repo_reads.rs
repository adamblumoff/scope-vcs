use super::{
    RepositoryStore, begin_metadata_read_snapshot, entities,
    landing_files::repository_landing_file,
    projection_encoding::ProjectionAudience,
    projection_read_models::{
        ProjectionFileLookup, live_projection_has_non_control_file_for_audience,
        load_live_projection_file_for_audience, load_live_projection_files_for_audience,
    },
    repository_from_model,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, prelude::Json,
};
use std::{collections::BTreeMap, sync::Arc};
use {
    crate::error::PostgresError,
    scope_domain::{
        landing_file::{REPOSITORY_LANDING_FILE_PATH, RepositoryLandingFile},
        policy::{Policy, Principal, PrincipalKind, ScopePath},
        projection_views::{
            ProjectionViewFile, ProjectionViewFileContent, has_visible_projected_non_control_files,
            projected_file_content as domain_projected_file_content,
            projected_files as domain_projected_files,
        },
        repository::access::{RepositoryAccess, RepositoryActor, repository_access_for_user_id},
        repository::collaboration::RepositoryMemberPermissions,
        repository::{RepoLifecycleState, Repository, repo_id},
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoSummaryRead {
    pub id: String,
    pub owner_handle: String,
    pub name: String,
    pub lifecycle_state: RepoLifecycleState,
    pub change_version: u64,
    pub access: RepositoryAccess,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerProfileRead {
    pub handle: String,
    pub repositories: Vec<RepoSummaryRead>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoLiveFileWithLandingContent {
    pub projected: ProjectionViewFileContent,
    pub landing_file: Option<RepositoryLandingFile>,
}

#[derive(Clone, Debug, FromQueryResult)]
struct RepoReadRow {
    id: String,
    owner_handle: String,
    name: String,
    owner_user_id: String,
    publication_state: String,
    change_version: i64,
    policy: Json,
}

impl RepositoryStore {
    pub async fn owner_profile(
        &self,
        handle: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<OwnerProfileRead>, PostgresError> {
        let handle = handle.to_string();
        let viewer_user_id = viewer_user_id.map(str::to_string);
        let db = Arc::clone(&self.db);
        let tx = begin_metadata_read_snapshot(db.as_ref()).await?;
        let profile = owner_profile_tx(&tx, &handle, viewer_user_id.as_deref()).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(profile)
    }

    pub async fn repo_summary(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<RepoSummaryRead>, PostgresError> {
        let owner = owner.to_string();
        let name = name.to_string();
        let viewer_user_id = viewer_user_id.map(str::to_string);
        let db = Arc::clone(&self.db);
        let tx = begin_metadata_read_snapshot(db.as_ref()).await?;
        let summary = repo_summary_tx(&tx, &owner, &name, viewer_user_id.as_deref()).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(summary)
    }

    pub async fn repo_live_files(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<Vec<ProjectionViewFile>>, PostgresError> {
        let owner = owner.to_string();
        let name = name.to_string();
        let viewer_user_id = viewer_user_id.map(str::to_string);
        let db = Arc::clone(&self.db);
        let tx = begin_metadata_read_snapshot(db.as_ref()).await?;
        let files = repo_live_files_tx(&tx, &owner, &name, viewer_user_id.as_deref()).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(files)
    }

    pub async fn repo_live_file_content(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
        path: &ScopePath,
    ) -> Result<Option<ProjectionViewFileContent>, PostgresError> {
        Ok(self
            .repo_live_file_with_landing_content(owner, name, viewer_user_id, path)
            .await?
            .map(|content| content.projected))
    }

    pub async fn repo_live_file_with_landing_content(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
        path: &ScopePath,
    ) -> Result<Option<RepoLiveFileWithLandingContent>, PostgresError> {
        let owner = owner.to_string();
        let name = name.to_string();
        let viewer_user_id = viewer_user_id.map(str::to_string);
        let path = path.clone();
        let db = Arc::clone(&self.db);
        let tx = begin_metadata_read_snapshot(db.as_ref()).await?;
        let content = repo_live_file_with_landing_content_tx(
            &tx,
            &owner,
            &name,
            viewer_user_id.as_deref(),
            &path,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(content)
    }
}

async fn owner_profile_tx<C>(
    conn: &C,
    handle: &str,
    viewer_user_id: Option<&str>,
) -> Result<Option<OwnerProfileRead>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(owner) = entities::user::Entity::find()
        .filter(entities::user::Column::Handle.eq(handle.to_string()))
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
    else {
        return Ok(None);
    };

    let rows = repo_read_rows_for_owner(conn, &owner.id).await?;
    let member_permissions = member_permissions_for_rows(conn, &rows, viewer_user_id).await?;
    let mut repositories = Vec::new();
    for row in rows {
        let permissions = member_permissions.get(&row.id).copied();
        let access = access_for_row(&row, viewer_user_id, permissions)?;
        if let Some(summary) = summary_for_viewer_row(conn, row, access).await? {
            repositories.push(summary);
        }
    }
    repositories.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(Some(OwnerProfileRead {
        handle: owner.handle,
        repositories,
    }))
}

async fn repo_summary_tx<C>(
    conn: &C,
    owner: &str,
    name: &str,
    viewer_user_id: Option<&str>,
) -> Result<Option<RepoSummaryRead>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(row) = repo_read_row_by_owner_name(conn, owner, name).await? else {
        return Ok(None);
    };
    let permissions = member_permissions_for_viewer(conn, &row, viewer_user_id).await?;
    let access = access_for_row(&row, viewer_user_id, permissions)?;
    summary_for_viewer_row(conn, row, access).await
}

async fn repo_live_files_tx<C>(
    conn: &C,
    owner: &str,
    name: &str,
    viewer_user_id: Option<&str>,
) -> Result<Option<Vec<ProjectionViewFile>>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(row) = repo_read_row_by_owner_name(conn, owner, name).await? else {
        return Ok(None);
    };
    let permissions = member_permissions_for_viewer(conn, &row, viewer_user_id).await?;
    let access = access_for_row(&row, viewer_user_id, permissions)?;
    let audience = live_projection_audience(access);

    if access.actor == RepositoryActor::Public && !public_surface_visible(conn, &row).await? {
        return Ok(None);
    }

    if let Some(files) =
        load_live_projection_files_for_audience(conn, &row.id, row.change_version()?, audience)
            .await?
    {
        return if row_is_readable(&row, access)? {
            Ok(Some(files))
        } else {
            Ok(None)
        };
    }

    let repo = hydrate_repo_from_row_id(conn, &row.id).await?;
    let principal = principal_for_access(viewer_user_id, access);
    if !row_is_readable(&row, access)? {
        return Ok(None);
    }
    Ok(Some(domain_projected_files(&repo, &principal)))
}

async fn repo_live_file_with_landing_content_tx<C>(
    conn: &C,
    owner: &str,
    name: &str,
    viewer_user_id: Option<&str>,
    path: &ScopePath,
) -> Result<Option<RepoLiveFileWithLandingContent>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(row) = repo_read_row_by_owner_name(conn, owner, name).await? else {
        return Ok(None);
    };
    let permissions = member_permissions_for_viewer(conn, &row, viewer_user_id).await?;
    let access = access_for_row(&row, viewer_user_id, permissions)?;
    if access.actor == RepositoryActor::Public && !public_surface_visible(conn, &row).await? {
        return Ok(None);
    }
    if !row_is_readable(&row, access)? {
        return Ok(None);
    }
    let audience = live_projection_audience(access);
    let lookup = load_live_projection_file_for_audience(
        conn,
        &row.id,
        row.change_version()?,
        audience,
        path,
    )
    .await?;
    let content = match lookup {
        ProjectionFileLookup::Found(content) => Some(content),
        ProjectionFileLookup::Missing => None,
        ProjectionFileLookup::NotReady => {
            let repo = hydrate_repo_from_row_id(conn, &row.id).await?;
            let principal = principal_for_access(viewer_user_id, access);
            domain_projected_file_content(&repo, &principal, path)
        }
    };
    let Some(projected) = content else {
        return Ok(None);
    };
    let landing_file = if path.as_str() == REPOSITORY_LANDING_FILE_PATH {
        repository_landing_file(conn, &row.id).await?
    } else {
        None
    };
    Ok(Some(RepoLiveFileWithLandingContent {
        projected,
        landing_file,
    }))
}

async fn repo_read_row_by_owner_name<C>(
    conn: &C,
    owner: &str,
    name: &str,
) -> Result<Option<RepoReadRow>, PostgresError>
where
    C: ConnectionTrait,
{
    let id = repo_id(owner, name);
    repo_read_query()
        .filter(entities::repository::Column::Id.eq(id))
        .into_model::<RepoReadRow>()
        .one(conn)
        .await
        .map_err(PostgresError::internal)
}

async fn repo_read_rows_for_owner<C>(
    conn: &C,
    user_id: &str,
) -> Result<Vec<RepoReadRow>, PostgresError>
where
    C: ConnectionTrait,
{
    repo_read_query()
        .filter(entities::repository::Column::OwnerUserId.eq(user_id.to_string()))
        .order_by_asc(entities::repository::Column::Id)
        .into_model::<RepoReadRow>()
        .all(conn)
        .await
        .map_err(PostgresError::internal)
}

fn repo_read_query() -> sea_orm::Select<entities::repository::Entity> {
    entities::repository::Entity::find()
        .select_only()
        .column(entities::repository::Column::Id)
        .column(entities::repository::Column::OwnerHandle)
        .column(entities::repository::Column::Name)
        .column(entities::repository::Column::OwnerUserId)
        .column(entities::repository::Column::PublicationState)
        .column(entities::repository::Column::ChangeVersion)
        .column(entities::repository::Column::Policy)
}

async fn member_permissions_for_viewer<C>(
    conn: &C,
    row: &RepoReadRow,
    viewer_user_id: Option<&str>,
) -> Result<Option<RepositoryMemberPermissions>, PostgresError>
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
        .map_err(PostgresError::internal)?
    else {
        return Ok(None);
    };
    Ok(Some(member.try_into_domain()?.permissions))
}

async fn member_permissions_for_rows<C>(
    conn: &C,
    rows: &[RepoReadRow],
    viewer_user_id: Option<&str>,
) -> Result<BTreeMap<String, RepositoryMemberPermissions>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(user_id) = viewer_user_id else {
        return Ok(BTreeMap::new());
    };
    let repo_ids = rows
        .iter()
        .filter(|row| row.owner_user_id != user_id)
        .map(|row| row.id.clone())
        .collect::<Vec<_>>();
    if repo_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let members = entities::repository_member::Entity::find()
        .filter(entities::repository_member::Column::RepoId.is_in(repo_ids))
        .filter(entities::repository_member::Column::UserId.eq(user_id.to_string()))
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;

    let mut permissions = BTreeMap::new();
    for member in members {
        permissions.insert(
            member.repo_id.clone(),
            member.try_into_domain()?.permissions,
        );
    }
    Ok(permissions)
}

async fn hydrate_repo_from_row_id<C>(conn: &C, repo_id: &str) -> Result<Repository, PostgresError>
where
    C: ConnectionTrait,
{
    let row = entities::repository::Entity::find_by_id(repo_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| {
            PostgresError::internal_message("repository row disappeared while reading")
        })?;
    repository_from_model(conn, row).await
}

async fn summary_for_viewer_row<C>(
    conn: &C,
    row: RepoReadRow,
    access: RepositoryAccess,
) -> Result<Option<RepoSummaryRead>, PostgresError>
where
    C: ConnectionTrait,
{
    if access.actor == RepositoryActor::Public && !public_surface_visible(conn, &row).await? {
        return Ok(None);
    }
    if !row_is_readable(&row, access)? {
        return Ok(None);
    }
    Ok(Some(summary_from_row(row, access)?))
}

fn summary_from_row(
    row: RepoReadRow,
    access: RepositoryAccess,
) -> Result<RepoSummaryRead, PostgresError> {
    let lifecycle_state = row.publication_state()?;
    let change_version = repo_change_version_for_access(row.change_version()?, access);
    Ok(RepoSummaryRead {
        id: row.id,
        owner_handle: row.owner_handle,
        name: row.name,
        lifecycle_state,
        change_version,
        access,
    })
}

fn access_for_row(
    row: &RepoReadRow,
    viewer_user_id: Option<&str>,
    member_permissions: Option<RepositoryMemberPermissions>,
) -> Result<RepositoryAccess, PostgresError> {
    let Some(user_id) = viewer_user_id else {
        return Ok(RepositoryAccess::public());
    };
    let publication_state = row.publication_state()?;
    Ok(repository_access_for_user_id(
        &row.owner_user_id,
        publication_state,
        member_permissions,
        user_id,
    ))
}

fn row_is_readable(row: &RepoReadRow, access: RepositoryAccess) -> Result<bool, PostgresError> {
    let publication_state = row.publication_state()?;
    let policy = row.policy()?;
    Ok(readable_from_facts(publication_state, &policy, access))
}

fn readable_from_facts(
    publication_state: RepoLifecycleState,
    policy: &Policy,
    access: RepositoryAccess,
) -> bool {
    let root = ScopePath::root();
    match access.actor {
        RepositoryActor::Owner => policy.can_read(&root, true),
        RepositoryActor::Member => {
            publication_state == RepoLifecycleState::Ready
                && policy.can_read(&root, access.can_read_private_files)
        }
        RepositoryActor::Public => publication_state == RepoLifecycleState::Ready,
    }
}

async fn public_surface_visible<C>(conn: &C, row: &RepoReadRow) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    if row.publication_state()? != RepoLifecycleState::Ready {
        return Ok(false);
    }
    if let Some(visible) = live_projection_has_non_control_file_for_audience(
        conn,
        &row.id,
        row.change_version()?,
        ProjectionAudience::Public,
    )
    .await?
    {
        return Ok(visible);
    }

    let repo = hydrate_repo_from_row_id(conn, &row.id).await?;
    Ok(has_visible_projected_non_control_files(
        &repo,
        &Principal::public(),
    ))
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

fn decode_enum<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, PostgresError> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .map_err(PostgresError::internal)
}

impl RepoReadRow {
    fn publication_state(&self) -> Result<RepoLifecycleState, PostgresError> {
        decode_enum(&self.publication_state)
    }

    fn policy(&self) -> Result<Policy, PostgresError> {
        serde_json::from_value(self.policy.clone()).map_err(PostgresError::internal)
    }

    fn change_version(&self) -> Result<u64, PostgresError> {
        u64::try_from(self.change_version).map_err(|_| {
            PostgresError::internal_message("repository change version cannot be negative")
        })
    }
}
