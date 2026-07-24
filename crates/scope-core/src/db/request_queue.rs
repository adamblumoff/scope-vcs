use super::{MetadataStore, RequestListRow, entities};
use crate::{
    domain::{
        requests::{
            REQUEST_LIST_MAX_PAGE_SIZE, RequestAudience, RequestQueueSection, RequestState,
        },
        store::{RepositoryAccess, RepositoryActor},
    },
    error::ApiError,
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect,
    sea_query::{Expr, Query, extension::postgres::PgExpr},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestQueueCursor {
    YourWork {
        updated_at_unix: u64,
        request_id: String,
    },
    Ready {
        snapshot_version: u64,
        stake_credits: u32,
        ready_at_unix: u64,
        request_id: String,
    },
    Completed {
        completed_at_unix: u64,
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
#[derive(FromQueryResult)]
struct ReadyQueueVersionDbRow {
    snapshot_version: i64,
}

// Ready-cycle versions are retained after exit, so MAX across all request history is
// the monotonic repository watermark. Domain facts couple publication to a non-null version.
async fn ready_queue_snapshot_version<C>(conn: &C, repo_id: &str) -> Result<u64, ApiError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find()
        .select_only()
        .expr_as(
            Expr::cust("COALESCE(MAX(ready_queue_version), 0)"),
            "snapshot_version",
        )
        .filter(entities::request::Column::RepoId.eq(repo_id))
        .into_model::<ReadyQueueVersionDbRow>()
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal_message("ready queue snapshot query returned no row"))
        .and_then(|row| entities::i64_to_u64(row.snapshot_version, "ready queue snapshot version"))
}

pub(super) async fn next_ready_queue_version<C>(conn: &C, repo_id: &str) -> Result<u64, ApiError>
where
    C: ConnectionTrait,
{
    ready_queue_snapshot_version(conn, repo_id)
        .await?
        .checked_add(1)
        .ok_or_else(|| ApiError::internal_message("ready queue version overflow"))
        .and_then(|version| {
            i64::try_from(version).map(|_| version).map_err(|_| {
                ApiError::internal_message("ready queue version exceeds PostgreSQL bigint range")
            })
        })
}

impl MetadataStore {
    pub async fn request_queue_page(
        &self,
        input: RequestQueuePageQuery<'_>,
    ) -> Result<Vec<RequestQueueRow>, ApiError> {
        if input.section == RequestQueueSection::YourWork && input.search.is_some() {
            return Err(ApiError::bad_request(
                "search is only supported for ready and completed requests",
            ));
        }
        let snapshot_version = ready_snapshot(&input, self).await?;
        let mut query = entities::request::Entity::find()
            .filter(entities::request::Column::RepoId.eq(input.repo_id));

        query = match input.section {
            RequestQueueSection::YourWork => {
                let viewer_user_id = input.viewer_user_id.ok_or_else(|| {
                    ApiError::bad_request("your work requires an authenticated viewer")
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
                    .filter(
                        Condition::any()
                            .add(
                                entities::request::Column::State
                                    .ne(entities::encode_enum(RequestState::Completed)?),
                            )
                            .add(entities::request::Column::FirstReadyAtUnix.is_not_null()),
                    )
            }
            RequestQueueSection::Ready => query
                .filter(
                    entities::request::Column::State
                        .eq(entities::encode_enum(RequestState::ReadyForReview)?),
                )
                .filter(
                    entities::request::Column::ReadyQueueVersion.lte(
                        i64::try_from(snapshot_version.expect("ready snapshot established"))
                            .map_err(ApiError::internal)?,
                    ),
                ),
            RequestQueueSection::Completed => query
                .filter(
                    entities::request::Column::State
                        .eq(entities::encode_enum(RequestState::Completed)?),
                )
                .filter(entities::request::Column::FirstReadyAtUnix.is_not_null()),
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
            RequestQueueSection::Ready => query
                .order_by_desc(entities::request::Column::CurrentStakeCredits)
                .order_by_asc(entities::request::Column::ReadyAtUnix)
                .order_by_asc(entities::request::Column::Id),
            RequestQueueSection::Completed => query
                .order_by_desc(entities::request::Column::CompletedAtUnix)
                .order_by_asc(entities::request::Column::Id),
        };

        query
            .limit(input.limit.min((REQUEST_LIST_MAX_PAGE_SIZE + 1) as u64))
            .all(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?
            .into_iter()
            .map(|row| {
                let request = row.try_into_domain()?;
                let cursor = cursor_for_request(input.section, snapshot_version, &request)?;
                Ok(RequestQueueRow {
                    request: RequestListRow::from(request),
                    cursor,
                })
            })
            .collect()
    }
}

async fn ready_snapshot(
    input: &RequestQueuePageQuery<'_>,
    store: &MetadataStore,
) -> Result<Option<u64>, ApiError> {
    match (input.section, input.after) {
        (
            RequestQueueSection::Ready,
            Some(RequestQueueCursor::Ready {
                snapshot_version, ..
            }),
        ) => Ok(Some(*snapshot_version)),
        (RequestQueueSection::Ready, None) => Ok(Some(
            ready_queue_snapshot_version(store.db.as_ref(), input.repo_id).await?,
        )),
        (RequestQueueSection::YourWork, None) | (RequestQueueSection::Completed, None) => Ok(None),
        (RequestQueueSection::YourWork, Some(RequestQueueCursor::YourWork { .. }))
        | (RequestQueueSection::Completed, Some(RequestQueueCursor::Completed { .. })) => Ok(None),
        _ => Err(ApiError::bad_request(
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
) -> Result<sea_orm::Select<entities::request::Entity>, ApiError> {
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
        RequestQueueCursor::Ready {
            stake_credits,
            ready_at_unix,
            request_id,
            ..
        } => {
            let stake_credits = i32::try_from(*stake_credits).map_err(ApiError::internal)?;
            let ready_at_unix = i64::try_from(*ready_at_unix).map_err(ApiError::internal)?;
            Condition::any()
                .add(entities::request::Column::CurrentStakeCredits.lt(stake_credits))
                .add(
                    Condition::all()
                        .add(entities::request::Column::CurrentStakeCredits.eq(stake_credits))
                        .add(entities::request::Column::ReadyAtUnix.gt(ready_at_unix)),
                )
                .add(
                    Condition::all()
                        .add(entities::request::Column::CurrentStakeCredits.eq(stake_credits))
                        .add(entities::request::Column::ReadyAtUnix.eq(ready_at_unix))
                        .add(entities::request::Column::Id.gt(request_id.as_str())),
                )
        }
        RequestQueueCursor::Completed {
            completed_at_unix,
            request_id,
        } => descending_time_cursor(
            entities::request::Column::CompletedAtUnix,
            *completed_at_unix,
            request_id,
        )?,
    };
    query = query.filter(condition);
    Ok(query)
}

fn descending_time_cursor(
    column: entities::request::Column,
    value: u64,
    request_id: &str,
) -> Result<Condition, ApiError> {
    let value = i64::try_from(value).map_err(ApiError::internal)?;
    Ok(Condition::any().add(column.lt(value)).add(
        Condition::all()
            .add(column.eq(value))
            .add(entities::request::Column::Id.gt(request_id)),
    ))
}

fn cursor_for_request(
    section: RequestQueueSection,
    snapshot_version: Option<u64>,
    request: &crate::domain::requests::Request,
) -> Result<RequestQueueCursor, ApiError> {
    match section {
        RequestQueueSection::YourWork => Ok(RequestQueueCursor::YourWork {
            updated_at_unix: request.updated_at_unix,
            request_id: request.id.clone(),
        }),
        RequestQueueSection::Ready => Ok(RequestQueueCursor::Ready {
            snapshot_version: snapshot_version.expect("ready snapshot established"),
            stake_credits: request.current_stake_credits,
            ready_at_unix: request.ready_at_unix.ok_or_else(|| {
                ApiError::internal_message("ready request is missing its ready time")
            })?,
            request_id: request.id.clone(),
        }),
        RequestQueueSection::Completed => Ok(RequestQueueCursor::Completed {
            completed_at_unix: request.completed_at_unix.ok_or_else(|| {
                ApiError::internal_message("completed request is missing its completion time")
            })?,
            request_id: request.id.clone(),
        }),
    }
}
