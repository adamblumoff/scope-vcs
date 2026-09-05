use super::{
    RequestListRow, RequestStore, entities,
    request_rows::{request_list_projection, request_list_rows},
};
use sea_orm::{
    ColumnTrait, Condition, QueryFilter, QueryOrder, QuerySelect,
    sea_query::{Expr, Query, extension::postgres::PgExpr},
};
use {
    crate::error::PostgresError,
    scope_domain::{
        repository::access::{RepositoryAccess, RepositoryActor},
        requests::{REQUEST_LIST_MAX_PAGE_SIZE, RequestAudience, RequestQueueSection},
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestQueueCursor {
    YourWork {
        updated_at_unix: u64,
        request_id: String,
    },
    Open {
        submitted_at_unix: u64,
        request_id: String,
    },
    Closed {
        closed_at_unix: u64,
        request_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestQueuePageQuery<'a> {
    pub repo_id: &'a str,
    pub section: RequestQueueSection,
    pub viewer_user_id: Option<&'a str>,
    pub access: RepositoryAccess,
    pub search: Option<&'a str>,
    pub after: Option<&'a RequestQueueCursor>,
    pub limit: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestQueueRow {
    pub request: RequestListRow,
    pub cursor: RequestQueueCursor,
}
impl RequestStore {
    pub async fn request_queue_page(
        &self,
        input: RequestQueuePageQuery<'_>,
    ) -> Result<Vec<RequestQueueRow>, PostgresError> {
        if input.section == RequestQueueSection::YourWork && input.search.is_some() {
            return Err(PostgresError::invalid_input(
                "search is only supported for open and closed requests",
            ));
        }
        ensure_cursor_section(&input)?;
        let mut query =
            request_list_projection().filter(entities::request::Column::RepoId.eq(input.repo_id));

        query = match input.section {
            RequestQueueSection::YourWork => {
                let viewer_user_id = input.viewer_user_id.ok_or_else(|| {
                    PostgresError::invalid_input("your work requires an authenticated viewer")
                })?;
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
                    .and_where(entities::request_invitee::Column::UserId.eq(viewer_user_id))
                    .to_owned();
                query
                    .filter(
                        Condition::any()
                            .add(entities::request::Column::AuthorUserId.eq(viewer_user_id))
                            .add(Expr::exists(invitee)),
                    )
                    .filter(entities::request::Column::SubmittedAtUnix.is_null())
                    .filter(entities::request::Column::ClosedAtUnix.is_null())
                    .filter(entities::request::Column::MergedAtUnix.is_null())
            }
            RequestQueueSection::Open => query
                .filter(entities::request::Column::SubmittedAtUnix.is_not_null())
                .filter(entities::request::Column::ClosedAtUnix.is_null())
                .filter(entities::request::Column::MergedAtUnix.is_null()),
            RequestQueueSection::Closed => query.filter(
                Condition::any()
                    .add(entities::request::Column::ClosedAtUnix.is_not_null())
                    .add(entities::request::Column::MergedAtUnix.is_not_null()),
            ),
        };

        if private_requests_hidden(input.access, input.search) {
            query = query.filter(
                entities::request::Column::Audience
                    .eq(entities::encode_enum(RequestAudience::Public)?),
            );
        }
        if let Some(search) = input.search {
            let pattern = format!("%{}%", escape_like_pattern(search));
            query = query.filter(
                Condition::any()
                    .add(Expr::col(entities::request::Column::Title).ilike(pattern.clone()))
                    .add(Expr::col(entities::request::Column::DescriptionMarkdown).ilike(pattern)),
            );
        }

        query = apply_cursor(query, input.after)?;
        query = match input.section {
            RequestQueueSection::YourWork => query
                .order_by_desc(entities::request::Column::UpdatedAtUnix)
                .order_by_asc(entities::request::Column::Id),
            RequestQueueSection::Open => query
                .order_by_asc(entities::request::Column::SubmittedAtUnix)
                .order_by_asc(entities::request::Column::Id),
            RequestQueueSection::Closed => query
                .order_by_desc(Expr::cust("COALESCE(closed_at_unix, merged_at_unix)"))
                .order_by_asc(entities::request::Column::Id),
        };

        let rows = request_list_rows(
            self.db.as_ref(),
            query.limit(input.limit.min((REQUEST_LIST_MAX_PAGE_SIZE + 1) as u64)),
        )
        .await?;
        rows.into_iter()
            .map(|request| {
                let cursor = cursor_for_request(input.section, &request)?;
                Ok(RequestQueueRow { request, cursor })
            })
            .collect()
    }
}

fn ensure_cursor_section(input: &RequestQueuePageQuery<'_>) -> Result<(), PostgresError> {
    match (input.section, input.after) {
        (_, None) => Ok(()),
        (RequestQueueSection::YourWork, Some(RequestQueueCursor::YourWork { .. }))
        | (RequestQueueSection::Open, Some(RequestQueueCursor::Open { .. }))
        | (RequestQueueSection::Closed, Some(RequestQueueCursor::Closed { .. })) => Ok(()),
        _ => Err(PostgresError::invalid_input(
            "request queue cursor section mismatch",
        )),
    }
}

fn private_requests_hidden(access: RepositoryAccess, search: Option<&str>) -> bool {
    search.is_some()
        || !matches!(
            access.actor,
            RepositoryActor::Owner | RepositoryActor::Member
        )
}

fn escape_like_pattern(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn apply_cursor(
    mut query: sea_orm::Select<entities::request::Entity>,
    after: Option<&RequestQueueCursor>,
) -> Result<sea_orm::Select<entities::request::Entity>, PostgresError> {
    let Some(after) = after else {
        return Ok(query);
    };
    let condition = match after {
        RequestQueueCursor::YourWork {
            updated_at_unix,
            request_id,
        } => descending_time_cursor(
            entities::request::Column::UpdatedAtUnix,
            *updated_at_unix,
            request_id,
        )?,
        RequestQueueCursor::Open {
            submitted_at_unix,
            request_id,
        } => ascending_time_cursor(
            entities::request::Column::SubmittedAtUnix,
            *submitted_at_unix,
            request_id,
        )?,
        RequestQueueCursor::Closed {
            closed_at_unix,
            request_id,
        } => {
            let value = i64::try_from(*closed_at_unix).map_err(PostgresError::internal)?;
            let terminal_time = Expr::cust("COALESCE(closed_at_unix, merged_at_unix)");
            // Bound the index scan before applying the ascending ID tie-break.
            Condition::all()
                .add(Expr::expr(terminal_time.clone()).lte(value))
                .add(
                    Condition::any()
                        .add(Expr::expr(terminal_time).lt(value))
                        .add(entities::request::Column::Id.gt(request_id)),
                )
        }
    };
    query = query.filter(condition);
    Ok(query)
}

fn descending_time_cursor(
    column: entities::request::Column,
    value: u64,
    request_id: &str,
) -> Result<Condition, PostgresError> {
    let value = i64::try_from(value).map_err(PostgresError::internal)?;
    Ok(Condition::all().add(column.lte(value)).add(
        Condition::any()
            .add(column.lt(value))
            .add(entities::request::Column::Id.gt(request_id)),
    ))
}

fn ascending_time_cursor(
    column: entities::request::Column,
    value: u64,
    request_id: &str,
) -> Result<Condition, PostgresError> {
    let value = i64::try_from(value).map_err(PostgresError::internal)?;
    Ok(Condition::all().add(
        Expr::tuple([
            Expr::col(column).into(),
            Expr::col(entities::request::Column::Id).into(),
        ])
        .gt(Expr::tuple([Expr::value(value), Expr::value(request_id)])),
    ))
}

fn cursor_for_request(
    section: RequestQueueSection,
    request: &RequestListRow,
) -> Result<RequestQueueCursor, PostgresError> {
    match section {
        RequestQueueSection::YourWork => Ok(RequestQueueCursor::YourWork {
            updated_at_unix: request.updated_at_unix,
            request_id: request.id.clone(),
        }),
        RequestQueueSection::Open => Ok(RequestQueueCursor::Open {
            submitted_at_unix: request.submitted_at_unix.ok_or_else(|| {
                PostgresError::internal_message("open request is missing its submission time")
            })?,
            request_id: request.id.clone(),
        }),
        RequestQueueSection::Closed => Ok(RequestQueueCursor::Closed {
            closed_at_unix: request
                .closed_at_unix
                .or(request.merged_at_unix)
                .ok_or_else(|| {
                    PostgresError::internal_message("terminal request is missing its terminal time")
                })?,
            request_id: request.id.clone(),
        }),
    }
}
