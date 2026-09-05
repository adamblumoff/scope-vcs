use super::entities;
use super::object_references::{delete_object_reference, replace_object_reference};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, FromQueryResult,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect,
    sea_query::{Expr, Query},
};
use {
    crate::error::PostgresError,
    scope_domain::repository::access::RepositoryAccess,
    scope_domain::requests::{
        REQUEST_LIST_MAX_PAGE_SIZE, Request, RequestActorRole, RequestAudience, RequestEvent,
        RequestListPredicate, RequestState, request_list_predicate,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestListRow {
    pub id: String,
    pub name: String,
    pub title: String,
    pub author_role: RequestActorRole,
    pub audience: RequestAudience,
    pub head_oid: String,
    pub state: RequestState,
    pub submitted_at_unix: Option<u64>,
    pub closed_at_unix: Option<u64>,
    pub merged_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub has_git_snapshot: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestListPageQuery<'a> {
    pub repo_id: &'a str,
    pub viewer_user_id: Option<&'a str>,
    pub access: RepositoryAccess,
    pub after_id: Option<&'a str>,
    pub limit: u64,
}

#[derive(Clone, Debug, FromQueryResult)]
struct RequestListModel {
    id: String,
    name: String,
    title: String,
    author_role: String,
    audience: String,
    head_oid: String,
    submitted_at_unix: Option<i64>,
    closed_at_unix: Option<i64>,
    merged_at_unix: Option<i64>,
    updated_at_unix: i64,
    has_git_snapshot: bool,
}

impl RequestListModel {
    fn try_into_read_model(self) -> Result<RequestListRow, PostgresError> {
        let state = if self.merged_at_unix.is_some() {
            RequestState::Merged
        } else if self.closed_at_unix.is_some() {
            RequestState::Closed
        } else if self.submitted_at_unix.is_some() {
            RequestState::Open
        } else {
            RequestState::Draft
        };
        Ok(RequestListRow {
            id: self.id,
            name: self.name,
            title: self.title,
            author_role: entities::decode_enum(self.author_role)?,
            audience: entities::decode_enum(self.audience)?,
            head_oid: self.head_oid,
            state,
            submitted_at_unix: self
                .submitted_at_unix
                .map(|value| entities::i64_to_u64(value, "request submission time"))
                .transpose()?,
            closed_at_unix: self
                .closed_at_unix
                .map(|value| entities::i64_to_u64(value, "request close time"))
                .transpose()?,
            merged_at_unix: self
                .merged_at_unix
                .map(|value| entities::i64_to_u64(value, "request merge time"))
                .transpose()?,
            updated_at_unix: entities::i64_to_u64(self.updated_at_unix, "request update time")?,
            has_git_snapshot: self.has_git_snapshot,
        })
    }
}

pub async fn request_list_page<C>(
    conn: &C,
    input: RequestListPageQuery<'_>,
) -> Result<Vec<RequestListRow>, PostgresError>
where
    C: ConnectionTrait,
{
    request_list_rows(conn, request_list_select(&input)?).await
}

pub(super) async fn request_list_rows<C>(
    conn: &C,
    query: sea_orm::Select<entities::request::Entity>,
) -> Result<Vec<RequestListRow>, PostgresError>
where
    C: ConnectionTrait,
{
    query
        .into_model::<RequestListModel>()
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(RequestListModel::try_into_read_model)
        .collect()
}

fn request_list_select(
    input: &RequestListPageQuery<'_>,
) -> Result<sea_orm::Select<entities::request::Entity>, PostgresError> {
    let mut query = request_list_projection()
        .filter(entities::request::Column::RepoId.eq(input.repo_id))
        .order_by_asc(entities::request::Column::Id)
        .limit(input.limit.min((REQUEST_LIST_MAX_PAGE_SIZE + 1) as u64));
    if let Some(after_id) = input.after_id {
        query = query.filter(entities::request::Column::Id.gt(after_id));
    }
    query = query.filter(request_list_condition(&request_list_predicate(
        input.access,
        input.viewer_user_id,
    ))?);
    Ok(query)
}

pub(super) fn request_list_condition(
    predicate: &RequestListPredicate<'_>,
) -> Result<Condition, PostgresError> {
    match predicate {
        RequestListPredicate::All(predicates) => predicates
            .iter()
            .try_fold(Condition::all(), |condition, predicate| {
                Ok(condition.add(request_list_condition(predicate)?))
            }),
        RequestListPredicate::Any(predicates) => predicates
            .iter()
            .try_fold(Condition::any(), |condition, predicate| {
                Ok(condition.add(request_list_condition(predicate)?))
            }),
        RequestListPredicate::Audience(audience) => Ok(Condition::all()
            .add(entities::request::Column::Audience.eq(entities::encode_enum(*audience)?))),
        RequestListPredicate::Submitted => {
            Ok(Condition::all().add(entities::request::Column::SubmittedAtUnix.is_not_null()))
        }
        RequestListPredicate::Author(viewer_user_id) => {
            Ok(Condition::all().add(entities::request::Column::AuthorUserId.eq(*viewer_user_id)))
        }
        RequestListPredicate::Invitee(viewer_user_id) => {
            let invitee = Query::select()
                .expr(Expr::val(1))
                .from(entities::request_invitee::Entity)
                .and_where(
                    Expr::col((
                        entities::request_invitee::Entity,
                        entities::request_invitee::Column::RequestId,
                    ))
                    .equals((entities::request::Entity, entities::request::Column::Id)),
                )
                .and_where(entities::request_invitee::Column::UserId.eq(*viewer_user_id))
                .to_owned();
            Ok(Condition::all().add(Expr::exists(invitee)))
        }
    }
}

pub(super) fn request_list_projection() -> sea_orm::Select<entities::request::Entity> {
    entities::request::Entity::find()
        .select_only()
        .column(entities::request::Column::Id)
        .column(entities::request::Column::Name)
        .column(entities::request::Column::Title)
        .column(entities::request::Column::AuthorRole)
        .column(entities::request::Column::Audience)
        .column(entities::request::Column::HeadOid)
        .column(entities::request::Column::SubmittedAtUnix)
        .column(entities::request::Column::ClosedAtUnix)
        .column(entities::request::Column::MergedAtUnix)
        .column(entities::request::Column::UpdatedAtUnix)
        .expr_as(
            Expr::col(entities::request::Column::GitSnapshot).is_not_null(),
            "has_git_snapshot",
        )
}

pub async fn request_by_id<C>(conn: &C, request_id: &str) -> Result<Option<Request>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find_by_id(request_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .map(entities::request::Model::try_into_domain)
        .transpose()
}

pub async fn request_by_name<C>(
    conn: &C,
    repo_id: &str,
    request_name: &str,
) -> Result<Option<Request>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find()
        .filter(entities::request::Column::RepoId.eq(repo_id.to_string()))
        .filter(entities::request::Column::Name.eq(request_name.to_string()))
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .map(entities::request::Model::try_into_domain)
        .transpose()
}

pub async fn requests_by_repo_id<C>(conn: &C, repo_id: &str) -> Result<Vec<Request>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find()
        .filter(entities::request::Column::RepoId.eq(repo_id.to_string()))
        .order_by_asc(entities::request::Column::Id)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::request::Model::try_into_domain)
        .collect()
}

pub async fn requests_by_repo_author<C>(
    conn: &C,
    repo_id: &str,
    author_user_id: &str,
) -> Result<Vec<Request>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find()
        .filter(entities::request::Column::RepoId.eq(repo_id.to_string()))
        .filter(entities::request::Column::AuthorUserId.eq(author_user_id.to_string()))
        .order_by_asc(entities::request::Column::Id)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::request::Model::try_into_domain)
        .collect()
}

pub async fn request_events_by_request_id<C>(
    conn: &C,
    request_id: &str,
) -> Result<Vec<RequestEvent>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request_event::Entity::find()
        .filter(entities::request_event::Column::RequestId.eq(request_id.to_string()))
        .order_by_asc(entities::request_event::Column::CreatedAtUnix)
        .order_by_asc(entities::request_event::Column::Position)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::request_event::Model::try_into_domain)
        .collect()
}

pub async fn request_events_after_position<C>(
    conn: &C,
    request_id: &str,
    after_position: u64,
    limit: u64,
) -> Result<Vec<RequestEvent>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request_event::Entity::find()
        .filter(entities::request_event::Column::RequestId.eq(request_id))
        .filter(
            entities::request_event::Column::Position
                .gt(i64::try_from(after_position).map_err(PostgresError::internal)?),
        )
        .order_by_asc(entities::request_event::Column::Position)
        .limit(limit)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::request_event::Model::try_into_domain)
        .collect()
}

pub async fn latest_request_events<C>(
    conn: &C,
    request_id: &str,
    limit: u64,
) -> Result<Vec<RequestEvent>, PostgresError>
where
    C: ConnectionTrait,
{
    let mut events = entities::request_event::Entity::find()
        .filter(entities::request_event::Column::RequestId.eq(request_id))
        .order_by_desc(entities::request_event::Column::Position)
        .limit(limit)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::request_event::Model::try_into_domain)
        .collect::<Result<Vec<_>, _>>()?;
    events.reverse();
    Ok(events)
}

pub async fn request_event_by_id<C>(
    conn: &C,
    event_id: &str,
) -> Result<Option<RequestEvent>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request_event::Entity::find_by_id(event_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .map(entities::request_event::Model::try_into_domain)
        .transpose()
}

pub async fn insert_request_row<C>(conn: &C, request: &Request) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    entities::request::Model::from_domain(request)?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(PostgresError::internal)?;
    replace_object_reference(
        conn,
        "request_snapshot",
        &request.id,
        request.git_snapshot.as_ref(),
    )
    .await?;
    Ok(())
}

pub async fn delete_request_rows<C>(conn: &C, request_id: &str) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    entities::request_invitee::Entity::delete_many()
        .filter(entities::request_invitee::Column::RequestId.eq(request_id.to_string()))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    entities::request_event::Entity::delete_many()
        .filter(entities::request_event::Column::RequestId.eq(request_id.to_string()))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    entities::request::Entity::delete_by_id(request_id.to_string())
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    delete_object_reference(conn, "request_snapshot", request_id).await?;
    Ok(())
}

pub async fn save_request_row<C>(conn: &C, request: &Request) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let row = entities::request::Model::from_domain(request)?;
    let result = entities::request::Entity::update_many()
        .filter(entities::request::Column::Id.eq(row.id))
        .col_expr(entities::request::Column::Title, Expr::value(row.title))
        .col_expr(
            entities::request::Column::DescriptionMarkdown,
            Expr::value(row.description_markdown),
        )
        .col_expr(
            entities::request::Column::HeadOid,
            Expr::value(row.head_oid),
        )
        .col_expr(
            entities::request::Column::GitSnapshot,
            Expr::value(row.git_snapshot),
        )
        .col_expr(
            entities::request::Column::ActivityVersion,
            Expr::value(row.activity_version),
        )
        .col_expr(
            entities::request::Column::SubmittedAtUnix,
            Expr::value(row.submitted_at_unix),
        )
        .col_expr(
            entities::request::Column::ClosedAtUnix,
            Expr::value(row.closed_at_unix),
        )
        .col_expr(
            entities::request::Column::ClosedByUserId,
            Expr::value(row.closed_by_user_id),
        )
        .col_expr(
            entities::request::Column::MergedAtUnix,
            Expr::value(row.merged_at_unix),
        )
        .col_expr(
            entities::request::Column::MergedByUserId,
            Expr::value(row.merged_by_user_id),
        )
        .col_expr(
            entities::request::Column::MergedHeadOid,
            Expr::value(row.merged_head_oid),
        )
        .col_expr(
            entities::request::Column::MergedMainOid,
            Expr::value(row.merged_main_oid),
        )
        .col_expr(
            entities::request::Column::UpdatedAtUnix,
            Expr::value(row.updated_at_unix),
        )
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    if result.rows_affected == 0 {
        return Err(PostgresError::internal_message(
            "request row missing during update",
        ));
    }
    replace_object_reference(
        conn,
        "request_snapshot",
        &request.id,
        request.git_snapshot.as_ref(),
    )
    .await?;
    Ok(())
}

pub async fn insert_request_event_row<C>(
    conn: &C,
    event: &RequestEvent,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    entities::request_event::Model::from_domain(event)?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(PostgresError::internal)?;
    Ok(())
}

#[cfg(test)]
mod request_list_tests {
    use super::*;
    use sea_orm::{DatabaseBackend, QueryTrait};

    #[test]
    fn request_list_query_projects_only_bounded_list_facts() {
        let query = request_list_select(&RequestListPageQuery {
            repo_id: "repo-1",
            viewer_user_id: Some("viewer-1"),
            access: RepositoryAccess::public(),
            after_id: Some("request-10"),
            limit: u64::MAX,
        })
        .unwrap();
        let sql = query.build(DatabaseBackend::Postgres).to_string();
        let projection = sql.split(" FROM ").next().unwrap();

        assert!(!projection.contains("description_markdown"));
        assert_eq!(projection.matches("\"git_snapshot\"").count(), 1);
        assert!(projection.contains("git_snapshot\" IS NOT NULL"));
        assert!(projection.contains("AS \"has_git_snapshot\""));
        assert!(sql.contains("EXISTS"));
        assert!(sql.contains("author_user_id"));
        assert!(sql.contains("submitted_at_unix"));
        assert!(sql.contains("ORDER BY \"scope_requests\".\"id\" ASC"));
        assert!(sql.ends_with("LIMIT 101"));
    }
}
