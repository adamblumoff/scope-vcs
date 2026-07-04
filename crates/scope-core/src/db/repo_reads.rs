use super::{
    MetadataStore, MetadataStoreInner, begin_metadata_read_snapshot, entities,
    projection_read_models::{
        load_live_projection_file_count_for_audience, load_live_projection_files_for_audience,
    },
    repository_from_model, run_api_db_on,
};
use crate::{
    domain::{
        policy::{Policy, Principal, PrincipalKind, ScopePath, Visibility},
        projection_views::{
            ProjectionViewFile, has_visible_projected_history,
            projected_files as domain_projected_files,
        },
        store::{
            RepoPublicationState, RepoSettings, RepositoryAccess, RepositoryActor,
            RepositoryMemberPermissions, StoredRepository, repo_id,
        },
    },
    error::ApiError,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, prelude::Json, sea_query::Expr,
};
use std::{collections::BTreeMap, sync::Arc};

const LIVE_PRIVATE_AUDIENCE: &str = "private";
const LIVE_PUBLIC_AUDIENCE: &str = "public";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoSummaryRead {
    pub id: String,
    pub owner_handle: String,
    pub name: String,
    pub lifecycle_state: RepoPublicationState,
    pub default_visibility: Visibility,
    pub change_version: u64,
    pub access: RepositoryAccess,
    pub pending_import_pending: bool,
    pub staged_update_pending: bool,
    pub push_blocked_by_staged_update: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoSettingsRead {
    pub default_new_file_visibility: Visibility,
    pub review_pushes_before_applying: bool,
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
    pending_import_pending: bool,
    staged_update_pending: bool,
}

impl MetadataStore {
    pub fn repo_summaries_for_user(&self, user_id: &str) -> Result<Vec<RepoSummaryRead>, ApiError> {
        let user_id = user_id.to_string();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = begin_metadata_read_snapshot(db.as_ref()).await?;
                    let summaries = repo_summaries_for_user_tx(&tx, &user_id).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(summaries)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(memory) => {
                let catalog = memory
                    .catalog
                    .lock()
                    .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?;
                let mut summaries = catalog
                    .repositories_for_user(&user_id)
                    .into_iter()
                    .filter_map(|repo| repo_summary_for_user(repo, &user_id))
                    .collect::<Vec<_>>();
                summaries.sort_by(|left, right| left.id.cmp(&right.id));
                Ok(summaries)
            }
        }
    }

    pub fn repo_summary(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<RepoSummaryRead>, ApiError> {
        let owner = owner.to_string();
        let name = name.to_string();
        let viewer_user_id = viewer_user_id.map(str::to_string);
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = begin_metadata_read_snapshot(db.as_ref()).await?;
                    let summary =
                        repo_summary_tx(&tx, &owner, &name, viewer_user_id.as_deref()).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(summary)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(memory) => {
                let catalog = memory
                    .catalog
                    .lock()
                    .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?;
                Ok(catalog
                    .repository(&owner, &name)
                    .and_then(|repo| repo_summary_for_viewer(repo, viewer_user_id.as_deref())))
            }
        }
    }

    pub fn repo_settings(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<RepoSettingsRead>, ApiError> {
        let owner = owner.to_string();
        let name = name.to_string();
        let viewer_user_id = viewer_user_id.map(str::to_string);
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = begin_metadata_read_snapshot(db.as_ref()).await?;
                    let settings =
                        repo_settings_tx(&tx, &owner, &name, viewer_user_id.as_deref()).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(settings)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(memory) => {
                let catalog = memory
                    .catalog
                    .lock()
                    .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?;
                let Some(repo) = catalog.repository(&owner, &name) else {
                    return Ok(None);
                };
                if viewer_user_id
                    .as_deref()
                    .is_some_and(|user_id| repo.is_owner_user(user_id))
                {
                    if !repo_is_readable_for_viewer(repo, viewer_user_id.as_deref()) {
                        return Ok(None);
                    }
                    return Ok(Some(repo_settings_read(
                        repo.record.default_visibility,
                        repo.settings,
                    )));
                }
                if repo_is_readable_for_viewer(repo, viewer_user_id.as_deref()) {
                    return Err(ApiError::forbidden("owner role required"));
                }
                Ok(None)
            }
        }
    }

    pub fn repo_live_files(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<Vec<ProjectionViewFile>>, ApiError> {
        let owner = owner.to_string();
        let name = name.to_string();
        let viewer_user_id = viewer_user_id.map(str::to_string);
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = begin_metadata_read_snapshot(db.as_ref()).await?;
                    let files =
                        repo_live_files_tx(&tx, &owner, &name, viewer_user_id.as_deref()).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(files)
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(memory) => {
                let catalog = memory
                    .catalog
                    .lock()
                    .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?;
                let Some(repo) = catalog.repository(&owner, &name) else {
                    return Ok(None);
                };
                if !repo_is_readable_for_viewer(repo, viewer_user_id.as_deref()) {
                    return Ok(None);
                }
                let principal = principal_for_repo_viewer(repo, viewer_user_id.as_deref());
                Ok(Some(domain_projected_files(repo, &principal)))
            }
        }
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

async fn repo_settings_tx<C>(
    conn: &C,
    owner: &str,
    name: &str,
    viewer_user_id: Option<&str>,
) -> Result<Option<RepoSettingsRead>, ApiError>
where
    C: ConnectionTrait,
{
    let Some(row) = repo_read_row_by_owner_name(conn, owner, name).await? else {
        return Ok(None);
    };
    if viewer_user_id.is_some_and(|user_id| user_id == row.owner_user_id) {
        let repo_id = row.id.clone();
        let default_visibility = row.default_visibility()?;
        if !row.policy()?.can_read(&ScopePath::root(), true) {
            return Ok(None);
        }
        let settings = entities::repository_setting::Entity::find_by_id(repo_id)
            .one(conn)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::internal_message("repository settings row is missing"))?;
        return Ok(Some(repo_settings_read(
            default_visibility,
            settings.into_domain(),
        )));
    }

    let permissions = member_permissions_for_viewer(conn, &row, viewer_user_id).await?;
    let access = access_for_row(&row, viewer_user_id, permissions)?;
    if row_is_readable_for_viewer(conn, &row, viewer_user_id, access).await? {
        return Err(ApiError::forbidden("owner role required"));
    }
    Ok(None)
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
        load_live_projection_files_for_audience(conn, &row.id, row.change_version(), audience)
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
        .column_as(
            Expr::col(entities::repository::Column::PendingImport).is_not_null(),
            "pending_import_pending",
        )
        .column_as(
            Expr::col(entities::repository::Column::StagedUpdate).is_not_null(),
            "staged_update_pending",
        )
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
            row.change_version(),
            LIVE_PUBLIC_AUDIENCE,
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
        row.change_version(),
        LIVE_PUBLIC_AUDIENCE,
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
    let change_version = repo_change_version_for_access(row.change_version(), access);
    let pending_import_pending =
        lifecycle_state == RepoPublicationState::Unpublished && row.pending_import_pending;
    Ok(RepoSummaryRead {
        id: row.id,
        owner_handle: row.owner_handle,
        name: row.name,
        lifecycle_state,
        default_visibility,
        change_version,
        access,
        pending_import_pending,
        staged_update_pending: can_review_staged_update(access) && row.staged_update_pending,
        push_blocked_by_staged_update: access.can_push && row.staged_update_pending,
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
            can_update_repo_settings: true,
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
        can_update_repo_settings: false,
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

#[cfg(any(test, feature = "memory-metadata"))]
fn repo_summary_for_user(repo: &StoredRepository, user_id: &str) -> Option<RepoSummaryRead> {
    let access = repo.access_for_user_id(user_id);
    if access.actor == RepositoryActor::Public {
        return None;
    }
    let lifecycle_allows_read = repo.record.publication_state == RepoPublicationState::Published
        || access.actor == RepositoryActor::Owner;
    if !lifecycle_allows_read
        || !repo
            .policy
            .can_read(&ScopePath::root(), access.can_read_private_files)
    {
        return None;
    }
    Some(summary_from_repo(repo, access))
}

#[cfg(any(test, feature = "memory-metadata"))]
fn repo_summary_for_viewer(
    repo: &StoredRepository,
    viewer_user_id: Option<&str>,
) -> Option<RepoSummaryRead> {
    let principal = principal_for_repo_viewer(repo, viewer_user_id);
    if !repo_is_readable_for_principal(repo, &principal) {
        return None;
    }
    Some(summary_from_repo(
        repo,
        repo.access_for_principal(&principal),
    ))
}

#[cfg(any(test, feature = "memory-metadata"))]
fn summary_from_repo(repo: &StoredRepository, access: RepositoryAccess) -> RepoSummaryRead {
    RepoSummaryRead {
        id: repo.record.id.clone(),
        owner_handle: repo.record.owner_handle.clone(),
        name: repo.record.name.clone(),
        lifecycle_state: repo.record.publication_state,
        default_visibility: repo.record.default_visibility,
        change_version: repo_change_version_for_access(repo.record.change_version, access),
        access,
        pending_import_pending: repo.has_pending_import_review(),
        staged_update_pending: can_review_staged_update(access) && repo.staged_update.is_some(),
        push_blocked_by_staged_update: access.can_push && repo.staged_update.is_some(),
    }
}

#[cfg(any(test, feature = "memory-metadata"))]
fn repo_is_readable_for_viewer(repo: &StoredRepository, viewer_user_id: Option<&str>) -> bool {
    let principal = principal_for_repo_viewer(repo, viewer_user_id);
    repo_is_readable_for_principal(repo, &principal)
}

#[cfg(any(test, feature = "memory-metadata"))]
fn repo_is_readable_for_principal(repo: &StoredRepository, principal: &Principal) -> bool {
    let access = repo.access_for_principal(principal);
    readable_from_facts(
        repo.record.publication_state,
        &repo.policy,
        access.actor == RepositoryActor::Public,
        access,
        repo.record.publication_state == RepoPublicationState::Published
            && has_visible_projected_history(repo, principal),
    )
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

fn repo_settings_read(default_visibility: Visibility, settings: RepoSettings) -> RepoSettingsRead {
    RepoSettingsRead {
        default_new_file_visibility: default_visibility,
        review_pushes_before_applying: settings.review_pushes_before_applying,
    }
}

fn live_projection_audience(access: RepositoryAccess) -> &'static str {
    if access.actor != RepositoryActor::Public && access.can_read_private_files {
        LIVE_PRIVATE_AUDIENCE
    } else {
        LIVE_PUBLIC_AUDIENCE
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

#[cfg(any(test, feature = "memory-metadata"))]
fn principal_for_repo_viewer(repo: &StoredRepository, viewer_user_id: Option<&str>) -> Principal {
    let Some(user_id) = viewer_user_id else {
        return Principal::public();
    };
    if repo.is_owner_user(user_id) || repo.member_for_user(user_id).is_some() {
        return Principal {
            id: user_id.to_string(),
            kind: PrincipalKind::User,
        };
    }
    Principal::public()
}

fn repo_change_version_for_access(change_version: u64, access: RepositoryAccess) -> u64 {
    if access.actor != RepositoryActor::Public {
        change_version
    } else {
        0
    }
}

fn can_review_staged_update(access: RepositoryAccess) -> bool {
    access.can_apply_changes || access.can_change_file_visibility
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

    fn change_version(&self) -> u64 {
        self.change_version.max(0) as u64
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
        assert!(
            !sql.contains("\"scope_repositories\".\"pending_import\","),
            "narrow repo reads should only test pending import presence: {sql}"
        );
        assert!(
            !sql.contains("\"scope_repositories\".\"staged_update\","),
            "narrow repo reads should only test staged update presence: {sql}"
        );
        assert!(
            sql.contains("\"pending_import\" IS NOT NULL"),
            "pending import state should be a SQL null check: {sql}"
        );
        assert!(
            sql.contains("\"staged_update\" IS NOT NULL"),
            "staged update state should be a SQL null check: {sql}"
        );
    }
}
