use super::{RepositoryStore, begin_metadata_read_snapshot, entities};
use crate::error::PostgresError;
use scope_domain::repository::{
    RepoRecord,
    access::{RepositoryAccess, RepositoryAccessContext, repository_access_for_user_id},
    repo_id,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QuerySelect,
};

#[derive(FromQueryResult)]
struct AccessRow {
    id: String,
    incarnation_id: String,
    owner_handle: String,
    name: String,
    owner_user_id: String,
    publication_state: String,
    change_version: i64,
    root_visibility: String,
}

impl RepositoryStore {
    pub async fn repository_read_access(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<RepositoryAccessContext>, PostgresError> {
        for _ in 0..3 {
            let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
            let Some(context) =
                repository_access(&tx, &repo_id(owner, name), viewer_user_id).await?
            else {
                tx.commit().await.map_err(PostgresError::internal)?;
                return Ok(None);
            };
            let public_files_visible = if context.access.actor
                == scope_domain::repository::access::RepositoryActor::Public
                && context.record.lifecycle_state
                    == scope_domain::repository::RepoLifecycleState::Ready
            {
                let Some(view) = super::history_reads::history_view_metadata(
                    &tx,
                    &context.record.id,
                    context.record.change_version,
                    scope_domain::projection::ProjectionViewKey::Public,
                )
                .await?
                else {
                    tx.commit().await.map_err(PostgresError::internal)?;
                    self.ensure_history_view(&context.incarnation()).await?;
                    continue;
                };
                view.visible_files
            } else {
                false
            };
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(context.can_read(public_files_visible).then_some(context));
        }
        Err(PostgresError::conflict(
            "repository kept changing while reading access; retry",
        ))
    }

    pub async fn repository_access(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<RepositoryAccessContext>, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let context = repository_access(&tx, &repo_id(owner, name), viewer_user_id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(context)
    }
}

pub(super) async fn repository_access<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    viewer_user_id: Option<&str>,
) -> Result<Option<RepositoryAccessContext>, PostgresError> {
    use entities::repository::{Column, Entity};
    let Some(row) = Entity::find()
        .select_only()
        .columns([
            Column::Id, Column::IncarnationId, Column::OwnerHandle, Column::Name,
            Column::OwnerUserId, Column::PublicationState, Column::ChangeVersion,
        ])
        .expr_as(sea_orm::sea_query::Expr::cust(
            "COALESCE(jsonb_path_query_first(policy, '$.rules[*] ? (@.path == \"/\")')->>'visibility', policy->>'default_visibility')"
        ), "root_visibility")
        .filter(Column::Id.eq(repo_id))
        .into_model::<AccessRow>()
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
    else { return Ok(None); };
    let record = RepoRecord {
        id: row.id,
        incarnation_id: row.incarnation_id,
        owner_handle: row.owner_handle,
        name: row.name,
        owner_user_id: row.owner_user_id,
        lifecycle_state: entities::decode_enum(row.publication_state)?,
        change_version: entities::i64_to_u64(row.change_version, "repository change version")?,
    };
    let access = match viewer_user_id {
        None => RepositoryAccess::public(),
        Some(user_id) => {
            let permissions = if user_id == record.owner_user_id {
                None
            } else {
                entities::repository_member::Entity::find_by_id((
                    repo_id.to_string(),
                    user_id.to_string(),
                ))
                .one(conn)
                .await
                .map_err(PostgresError::internal)?
                .map(entities::repository_member::Model::try_into_domain)
                .transpose()?
                .map(|member| member.permissions)
            };
            repository_access_for_user_id(
                &record.owner_user_id,
                record.lifecycle_state,
                permissions,
                user_id,
            )
        }
    };
    Ok(Some(RepositoryAccessContext {
        record,
        access,
        root_visibility: entities::decode_enum(row.root_visibility)?,
    }))
}

impl RepositoryStore {
    pub async fn repository_content_source(
        &self,
        incarnation: &scope_domain::repository::RepositoryIncarnation,
    ) -> Result<
        (
            Option<scope_domain::repository::git::GitHead>,
            Vec<scope_domain::repository::git::GitPackSpan>,
        ),
        PostgresError,
    > {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let context = repository_access(&tx, incarnation.repository_id(), None)
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        if context.incarnation() != *incarnation {
            return Err(PostgresError::conflict("repository was recreated; retry"));
        }
        let head = entities::git_head::Entity::find_by_id(incarnation.repository_id())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .map(entities::git_head::Model::try_into_domain)
            .transpose()?;
        let spans =
            super::git_segments::load_git_pack_spans(&tx, incarnation.repository_id()).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok((head, spans))
    }
}

impl RepositoryStore {
    pub async fn repository_policy(
        &self,
        context: &RepositoryAccessContext,
    ) -> Result<scope_domain::policy::Policy, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let current = repository_access(&tx, &context.record.id, None)
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        ensure_current_context(context, &current)?;
        let policy = entities::repository::Entity::find_by_id(&context.record.id)
            .select_only()
            .column(entities::repository::Column::Policy)
            .into_tuple::<serde_json::Value>()
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        tx.commit().await.map_err(PostgresError::internal)?;
        serde_json::from_value(policy).map_err(PostgresError::internal)
    }

    pub async fn repository_main_oid(
        &self,
        context: &RepositoryAccessContext,
    ) -> Result<Option<String>, PostgresError> {
        self.repository_main_oid_from(context, false).await
    }

    pub async fn repository_committed_main_oid(
        &self,
        context: &RepositoryAccessContext,
    ) -> Result<Option<String>, PostgresError> {
        self.repository_main_oid_from(context, true).await
    }

    async fn repository_main_oid_from(
        &self,
        context: &RepositoryAccessContext,
        build_projection: bool,
    ) -> Result<Option<String>, PostgresError> {
        let audience = scope_domain::projection::ProjectionViewKey::from_access(context.access);
        for _ in 0..2 {
            let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
            let current = repository_access(&tx, &context.record.id, None)
                .await?
                .ok_or_else(|| PostgresError::not_found("repo not found"))?;
            ensure_current_context(context, &current)?;
            if context.access.can_read_private_files
                && let Some(head) = entities::git_head::Entity::find_by_id(&context.record.id)
                    .one(&tx)
                    .await
                    .map_err(PostgresError::internal)?
            {
                tx.commit().await.map_err(PostgresError::internal)?;
                return Ok(Some(head.head_oid));
            }
            if !build_projection {
                let oid = super::projection_read_models::live_projection_head_oid_for_frontier(
                    &tx,
                    &context.record.id,
                    context.record.change_version,
                    audience,
                )
                .await?;
                tx.commit().await.map_err(PostgresError::internal)?;
                return Ok(oid);
            }
            let metadata = super::history_reads::history_view_metadata(
                &tx,
                &context.record.id,
                context.record.change_version,
                audience,
            )
            .await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            if let Some(view) = metadata {
                return Ok(view.head_oid);
            }
            self.ensure_history_view(&context.incarnation()).await?;
        }
        Err(PostgresError::conflict(
            "repository changed while reading its head; retry",
        ))
    }
}

fn ensure_current_context(
    expected: &RepositoryAccessContext,
    current: &RepositoryAccessContext,
) -> Result<(), PostgresError> {
    if current.incarnation() != expected.incarnation()
        || current.record.change_version != expected.record.change_version
    {
        return Err(PostgresError::conflict(
            "repository changed while reading metadata; retry",
        ));
    }
    Ok(())
}
