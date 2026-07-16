use super::entities;
use crate::{
    domain::{
        requests::{
            RequestDiscussion, RequestDiscussionReadState, RequestDiscussionReply,
            RequestDiscussionStatus,
        },
        store::UserAccount,
    },
    error::ApiError,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, QuerySelect, Statement, sea_query::Expr,
};
use std::collections::{BTreeMap, BTreeSet};

pub async fn discussion_by_id<C>(conn: &C, id: &str) -> Result<Option<RequestDiscussion>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request_discussion::Entity::find_by_id(id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::request_discussion::Model::try_into_domain)
        .transpose()
}

pub async fn discussion_by_client_id<C>(
    conn: &C,
    request_id: &str,
    author_user_id: &str,
    client_discussion_id: &str,
) -> Result<Option<RequestDiscussion>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request_discussion::Entity::find()
        .filter(entities::request_discussion::Column::RequestId.eq(request_id))
        .filter(entities::request_discussion::Column::AuthorUserId.eq(author_user_id))
        .filter(entities::request_discussion::Column::ClientDiscussionId.eq(client_discussion_id))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::request_discussion::Model::try_into_domain)
        .transpose()
}

pub async fn discussions_page_for_request<C>(
    conn: &C,
    request_id: &str,
    status: Option<RequestDiscussionStatus>,
    recent: bool,
    snapshot_version: u64,
    cursor: Option<(u64, String)>,
    limit: u64,
) -> Result<Vec<(RequestDiscussion, u64)>, ApiError>
where
    C: ConnectionTrait,
{
    let snapshot = i64::try_from(snapshot_version).map_err(ApiError::internal)?;
    let status = status.map(enum_string).transpose()?;
    let (cursor_position, cursor_id) = match cursor {
        Some((position, id)) => (
            Some(i64::try_from(position).map_err(ApiError::internal)?),
            Some(id),
        ),
        None => (None, None),
    };
    let limit = i64::try_from(limit).map_err(ApiError::internal)?;
    let rows = conn
        .query_all(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
            WITH reply_positions AS (
                SELECT replies.discussion_id, MAX(replies.position) AS position
                FROM scope_request_discussion_replies replies
                JOIN scope_request_discussions discussions
                  ON discussions.id = replies.discussion_id
                WHERE discussions.request_id = $1
                  AND replies.position <= $3
                GROUP BY replies.discussion_id
            ),
            transition_history AS (
                SELECT
                    COALESCE(
                        events.payload -> 'DiscussionResolved' ->> 'discussion_id',
                        events.payload -> 'DiscussionReopened' ->> 'discussion_id'
                    ) AS discussion_id,
                    events.position,
                    events.kind
                FROM scope_request_events events
                WHERE events.request_id = $1
                  AND events.position <= $3
                  AND events.kind IN ('DiscussionResolved', 'DiscussionReopened')
            ),
            latest_transitions AS (
                SELECT DISTINCT ON (discussion_id)
                    discussion_id,
                    position,
                    kind
                FROM transition_history
                WHERE discussion_id IS NOT NULL
                ORDER BY discussion_id, position DESC
            ),
            snapshot_discussions AS (
                SELECT
                    discussions.id,
                    CASE WHEN $2::boolean THEN
                        GREATEST(
                            discussions.opened_position,
                            COALESCE(reply_positions.position, discussions.opened_position),
                            COALESCE(latest_transitions.position, discussions.opened_position)
                        )
                    ELSE discussions.opened_position END AS sort_position,
                    discussions.status AS current_status,
                    COALESCE(
                        CASE latest_transitions.kind
                            WHEN 'DiscussionResolved' THEN 'Resolved'
                            WHEN 'DiscussionReopened' THEN 'Open'
                        END,
                        'Open'
                    ) AS snapshot_status
                FROM scope_request_discussions discussions
                LEFT JOIN reply_positions
                  ON reply_positions.discussion_id = discussions.id
                LEFT JOIN latest_transitions
                  ON latest_transitions.discussion_id = discussions.id
                WHERE discussions.request_id = $1
                  AND discussions.opened_position <= $3
            )
            SELECT id, sort_position
            FROM snapshot_discussions
            WHERE (
                $4::text IS NULL
                OR (snapshot_status = $4 AND current_status = $4)
            )
              AND (
                  $5::bigint IS NULL
                  OR sort_position < $5
                  OR (sort_position = $5 AND id > $6)
              )
            ORDER BY sort_position DESC, id ASC
            LIMIT $7
            "#,
            vec![
                request_id.into(),
                recent.into(),
                snapshot.into(),
                status.into(),
                cursor_position.into(),
                cursor_id.into(),
                limit.into(),
            ],
        ))
        .await
        .map_err(ApiError::internal)?;
    let ordered = rows
        .into_iter()
        .map(|row| {
            let id = row
                .try_get::<String>("", "id")
                .map_err(ApiError::internal)?;
            let position = row
                .try_get::<i64>("", "sort_position")
                .map_err(ApiError::internal)?
                .try_into()
                .map_err(ApiError::internal)?;
            Ok((id, position))
        })
        .collect::<Result<Vec<(String, u64)>, ApiError>>()?;
    if ordered.is_empty() {
        return Ok(Vec::new());
    }
    let discussions = entities::request_discussion::Entity::find()
        .filter(
            entities::request_discussion::Column::Id
                .is_in(ordered.iter().map(|(id, _)| id.clone())),
        )
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|model| Ok((model.id.clone(), model.try_into_domain()?)))
        .collect::<Result<BTreeMap<_, _>, ApiError>>()?;
    ordered
        .into_iter()
        .map(|(id, position)| {
            discussions
                .get(&id)
                .cloned()
                .map(|discussion| (discussion, position))
                .ok_or_else(|| ApiError::internal_message("paged discussion disappeared"))
        })
        .collect()
}

pub async fn changed_discussions_for_request<C>(
    conn: &C,
    request_id: &str,
    after_position: u64,
    limit: u64,
) -> Result<Vec<RequestDiscussion>, ApiError>
where
    C: ConnectionTrait,
{
    let after_position = i64::try_from(after_position).map_err(ApiError::internal)?;
    entities::request_discussion::Entity::find()
        .filter(entities::request_discussion::Column::RequestId.eq(request_id))
        .filter(entities::request_discussion::Column::LastActivityPosition.gt(after_position))
        .order_by_asc(entities::request_discussion::Column::LastActivityPosition)
        .order_by_asc(entities::request_discussion::Column::Id)
        .limit(limit)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(entities::request_discussion::Model::try_into_domain)
        .collect()
}

pub async fn replies_for_discussion<C>(
    conn: &C,
    discussion_id: &str,
    before_position: Option<u64>,
    limit: u64,
) -> Result<Vec<RequestDiscussionReply>, ApiError>
where
    C: ConnectionTrait,
{
    let mut query = entities::request_discussion_reply::Entity::find()
        .filter(entities::request_discussion_reply::Column::DiscussionId.eq(discussion_id));
    if let Some(before) = before_position {
        query = query.filter(
            entities::request_discussion_reply::Column::Position
                .lt(i64::try_from(before).map_err(ApiError::internal)?),
        );
    }
    let mut replies = query
        .order_by_desc(entities::request_discussion_reply::Column::Position)
        .order_by_desc(entities::request_discussion_reply::Column::Id)
        .limit(limit)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(entities::request_discussion_reply::Model::try_into_domain)
        .collect::<Result<Vec<_>, _>>()?;
    replies.reverse();
    Ok(replies)
}

pub async fn reply_previews_for_discussions<C>(
    conn: &C,
    discussion_ids: &[String],
) -> Result<BTreeMap<String, (u64, Vec<RequestDiscussionReply>)>, ApiError>
where
    C: ConnectionTrait,
{
    if discussion_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let placeholders = (1..=discussion_ids.len())
        .map(|index| format!("${index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT id, discussion_id, position, author_user_id, body_markdown, \
         reply_to_reply_id, client_reply_id, created_at_unix, reply_count \
         FROM ( \
           SELECT replies.*, \
             COUNT(*) OVER (PARTITION BY discussion_id) AS reply_count, \
             ROW_NUMBER() OVER (PARTITION BY discussion_id ORDER BY position DESC, id DESC) AS row_number \
           FROM scope_request_discussion_replies replies \
           WHERE discussion_id IN ({placeholders}) \
         ) ranked \
         WHERE row_number <= 3 \
         ORDER BY discussion_id ASC, position ASC, id ASC"
    );
    let rows = conn
        .query_all(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            sql,
            discussion_ids.iter().cloned().map(Into::into),
        ))
        .await
        .map_err(ApiError::internal)?;
    let mut result = BTreeMap::<String, (u64, Vec<RequestDiscussionReply>)>::new();
    for row in rows {
        let discussion_id = row
            .try_get::<String>("", "discussion_id")
            .map_err(ApiError::internal)?;
        let model = entities::request_discussion_reply::Model {
            id: row.try_get("", "id").map_err(ApiError::internal)?,
            discussion_id: discussion_id.clone(),
            position: row.try_get("", "position").map_err(ApiError::internal)?,
            author_user_id: row
                .try_get("", "author_user_id")
                .map_err(ApiError::internal)?,
            body_markdown: row
                .try_get("", "body_markdown")
                .map_err(ApiError::internal)?,
            reply_to_reply_id: row
                .try_get("", "reply_to_reply_id")
                .map_err(ApiError::internal)?,
            client_reply_id: row
                .try_get("", "client_reply_id")
                .map_err(ApiError::internal)?,
            created_at_unix: row
                .try_get("", "created_at_unix")
                .map_err(ApiError::internal)?,
        };
        let count = row
            .try_get::<i64>("", "reply_count")
            .map_err(ApiError::internal)?
            .try_into()
            .map_err(ApiError::internal)?;
        let entry = result
            .entry(discussion_id)
            .or_insert_with(|| (count, Vec::new()));
        entry.1.push(model.try_into_domain()?);
    }
    Ok(result)
}

pub async fn unread_content_counts<C>(
    conn: &C,
    discussions: &[RequestDiscussion],
    read_states: &BTreeMap<String, RequestDiscussionReadState>,
) -> Result<BTreeMap<String, u64>, ApiError>
where
    C: ConnectionTrait,
{
    if discussions.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut values = Vec::with_capacity(discussions.len() * 2);
    let rows = discussions
        .iter()
        .enumerate()
        .map(|(index, discussion)| {
            let base = index * 2 + 1;
            values.push(discussion.id.clone().into());
            values.push(
                i64::try_from(
                    read_states
                        .get(&discussion.id)
                        .map(|state| state.read_through_position)
                        .unwrap_or(0),
                )
                .map_err(ApiError::internal)?
                .into(),
            );
            Ok(format!("(${base}, ${})", base + 1))
        })
        .collect::<Result<Vec<_>, ApiError>>()?
        .join(", ");
    let sql = format!(
        "WITH reads(discussion_id, read_position) AS (VALUES {rows}) \
         SELECT reads.discussion_id, COUNT(replies.id) AS unread_replies \
         FROM reads \
         LEFT JOIN scope_request_discussion_replies replies \
           ON replies.discussion_id = reads.discussion_id \
          AND replies.position > reads.read_position \
         GROUP BY reads.discussion_id"
    );
    let rows = conn
        .query_all(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .await
        .map_err(ApiError::internal)?;
    let mut result = rows
        .into_iter()
        .map(|row| {
            let id = row
                .try_get::<String>("", "discussion_id")
                .map_err(ApiError::internal)?;
            let count = row
                .try_get::<i64>("", "unread_replies")
                .map_err(ApiError::internal)?
                .try_into()
                .map_err(ApiError::internal)?;
            Ok((id, count))
        })
        .collect::<Result<BTreeMap<String, u64>, ApiError>>()?;
    for discussion in discussions {
        let read_position = read_states
            .get(&discussion.id)
            .map(|state| state.read_through_position)
            .unwrap_or(0);
        if discussion.opened_position > read_position {
            *result.entry(discussion.id.clone()).or_default() += 1;
        }
    }
    Ok(result)
}

pub async fn reply_by_id<C>(conn: &C, id: &str) -> Result<Option<RequestDiscussionReply>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request_discussion_reply::Entity::find_by_id(id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::request_discussion_reply::Model::try_into_domain)
        .transpose()
}

pub async fn reply_by_client_id<C>(
    conn: &C,
    discussion_id: &str,
    author_user_id: &str,
    client_reply_id: &str,
) -> Result<Option<RequestDiscussionReply>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request_discussion_reply::Entity::find()
        .filter(entities::request_discussion_reply::Column::DiscussionId.eq(discussion_id))
        .filter(entities::request_discussion_reply::Column::AuthorUserId.eq(author_user_id))
        .filter(entities::request_discussion_reply::Column::ClientReplyId.eq(client_reply_id))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::request_discussion_reply::Model::try_into_domain)
        .transpose()
}

pub async fn read_states_for_user<C>(
    conn: &C,
    discussion_ids: &[String],
    user_id: &str,
) -> Result<BTreeMap<String, RequestDiscussionReadState>, ApiError>
where
    C: ConnectionTrait,
{
    if discussion_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    entities::request_discussion_read_state::Entity::find()
        .filter(
            entities::request_discussion_read_state::Column::DiscussionId
                .is_in(discussion_ids.iter().cloned()),
        )
        .filter(entities::request_discussion_read_state::Column::UserId.eq(user_id))
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|row| {
            let state = row.try_into_domain()?;
            Ok((state.discussion_id.clone(), state))
        })
        .collect()
}

pub async fn read_state<C>(
    conn: &C,
    discussion_id: &str,
    user_id: &str,
) -> Result<Option<RequestDiscussionReadState>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request_discussion_read_state::Entity::find_by_id((
        discussion_id.to_string(),
        user_id.to_string(),
    ))
    .one(conn)
    .await
    .map_err(ApiError::internal)?
    .map(entities::request_discussion_read_state::Model::try_into_domain)
    .transpose()
}

pub async fn users_by_ids<C>(
    conn: &C,
    user_ids: impl IntoIterator<Item = String>,
) -> Result<BTreeMap<String, UserAccount>, ApiError>
where
    C: ConnectionTrait,
{
    let user_ids = user_ids.into_iter().collect::<BTreeSet<_>>();
    if user_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    entities::user::Entity::find()
        .filter(entities::user::Column::Id.is_in(user_ids))
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|row| {
            let user = row.try_into_domain()?;
            Ok((user.id.clone(), user))
        })
        .collect()
}

pub async fn insert_discussion<C>(conn: &C, value: &RequestDiscussion) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::request_discussion::Model::from_domain(value)?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

pub async fn save_discussion<C>(conn: &C, value: &RequestDiscussion) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let row = entities::request_discussion::Model::from_domain(value)?;
    let result = entities::request_discussion::Entity::update_many()
        .filter(entities::request_discussion::Column::Id.eq(row.id))
        .col_expr(
            entities::request_discussion::Column::LastActivityPosition,
            Expr::value(row.last_activity_position),
        )
        .col_expr(
            entities::request_discussion::Column::Status,
            Expr::value(row.status),
        )
        .col_expr(
            entities::request_discussion::Column::ResolvedAtUnix,
            Expr::value(row.resolved_at_unix),
        )
        .col_expr(
            entities::request_discussion::Column::ResolvedByUserId,
            Expr::value(row.resolved_by_user_id),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    if result.rows_affected != 1 {
        return Err(ApiError::internal_message(
            "request discussion missing during update",
        ));
    }
    Ok(())
}

pub async fn insert_reply<C>(conn: &C, value: &RequestDiscussionReply) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::request_discussion_reply::Model::from_domain(value)?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

pub async fn save_read_state<C>(
    conn: &C,
    value: &RequestDiscussionReadState,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let row = entities::request_discussion_read_state::Model::from_domain(value)?;
    entities::request_discussion_read_state::Entity::insert(row.into_active_model())
        .on_conflict(
            sea_orm::sea_query::OnConflict::columns([
                entities::request_discussion_read_state::Column::DiscussionId,
                entities::request_discussion_read_state::Column::UserId,
            ])
            .update_columns([
                entities::request_discussion_read_state::Column::ReadThroughPosition,
                entities::request_discussion_read_state::Column::UpdatedAtUnix,
            ])
            .to_owned(),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

fn enum_string<T: serde::Serialize>(value: T) -> Result<String, ApiError> {
    match serde_json::to_value(value).map_err(ApiError::internal)? {
        serde_json::Value::String(value) => Ok(value),
        _ => Err(ApiError::internal_message(
            "enum did not serialize to string",
        )),
    }
}
