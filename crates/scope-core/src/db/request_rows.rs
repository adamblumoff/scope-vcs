use super::entities;
use super::object_references::{delete_object_reference, replace_object_reference};
use crate::{
    domain::requests::{
        CreditLedgerEntry, CreditLedgerEntryKind, Request, RequestEvent, UserCreditAccount,
    },
    error::ApiError,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect, sea_query::Expr,
};
pub async fn request_by_id<C>(conn: &C, request_id: &str) -> Result<Option<Request>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find_by_id(request_id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::request::Model::try_into_domain)
        .transpose()
}

pub async fn request_by_name<C>(
    conn: &C,
    repo_id: &str,
    request_name: &str,
) -> Result<Option<Request>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find()
        .filter(entities::request::Column::RepoId.eq(repo_id.to_string()))
        .filter(entities::request::Column::Name.eq(request_name.to_string()))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::request::Model::try_into_domain)
        .transpose()
}

pub async fn requests_by_repo_id<C>(conn: &C, repo_id: &str) -> Result<Vec<Request>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find()
        .filter(entities::request::Column::RepoId.eq(repo_id.to_string()))
        .order_by_asc(entities::request::Column::Id)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(entities::request::Model::try_into_domain)
        .collect()
}

pub async fn requests_by_repo_author<C>(
    conn: &C,
    repo_id: &str,
    author_user_id: &str,
) -> Result<Vec<Request>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find()
        .filter(entities::request::Column::RepoId.eq(repo_id.to_string()))
        .filter(entities::request::Column::AuthorUserId.eq(author_user_id.to_string()))
        .order_by_asc(entities::request::Column::Id)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(entities::request::Model::try_into_domain)
        .collect()
}

pub async fn request_events_by_request_id<C>(
    conn: &C,
    request_id: &str,
) -> Result<Vec<RequestEvent>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request_event::Entity::find()
        .filter(entities::request_event::Column::RequestId.eq(request_id.to_string()))
        .order_by_asc(entities::request_event::Column::CreatedAtUnix)
        .order_by_asc(entities::request_event::Column::Position)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(entities::request_event::Model::try_into_domain)
        .collect()
}

pub async fn request_events_after_position<C>(
    conn: &C,
    request_id: &str,
    after_position: u64,
    limit: u64,
) -> Result<Vec<RequestEvent>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request_event::Entity::find()
        .filter(entities::request_event::Column::RequestId.eq(request_id))
        .filter(
            entities::request_event::Column::Position
                .gt(i64::try_from(after_position).map_err(ApiError::internal)?),
        )
        .order_by_asc(entities::request_event::Column::Position)
        .limit(limit)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(entities::request_event::Model::try_into_domain)
        .collect()
}

pub async fn latest_request_events<C>(
    conn: &C,
    request_id: &str,
    limit: u64,
) -> Result<Vec<RequestEvent>, ApiError>
where
    C: ConnectionTrait,
{
    let mut events = entities::request_event::Entity::find()
        .filter(entities::request_event::Column::RequestId.eq(request_id))
        .order_by_desc(entities::request_event::Column::Position)
        .limit(limit)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(entities::request_event::Model::try_into_domain)
        .collect::<Result<Vec<_>, _>>()?;
    events.reverse();
    Ok(events)
}

pub async fn request_event_by_id<C>(
    conn: &C,
    event_id: &str,
) -> Result<Option<RequestEvent>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request_event::Entity::find_by_id(event_id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::request_event::Model::try_into_domain)
        .transpose()
}

pub async fn credit_account_by_user_id<C>(
    conn: &C,
    user_id: &str,
) -> Result<Option<UserCreditAccount>, ApiError>
where
    C: ConnectionTrait,
{
    entities::user_credit_account::Entity::find_by_id(user_id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::user_credit_account::Model::try_into_domain)
        .transpose()
}

pub async fn credit_ledger_entry_by_id<C>(
    conn: &C,
    entry_id: &str,
) -> Result<Option<CreditLedgerEntry>, ApiError>
where
    C: ConnectionTrait,
{
    entities::credit_ledger_entry::Entity::find_by_id(entry_id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::credit_ledger_entry::Model::try_into_domain)
        .transpose()
}

pub async fn request_stake_debit_entry_for_request_id<C>(
    conn: &C,
    request_id: &str,
) -> Result<Option<CreditLedgerEntry>, ApiError>
where
    C: ConnectionTrait,
{
    entities::credit_ledger_entry::Entity::find()
        .filter(entities::credit_ledger_entry::Column::RequestId.eq(Some(request_id.to_string())))
        .filter(
            entities::credit_ledger_entry::Column::Kind.eq(encode_credit_ledger_entry_kind(
                CreditLedgerEntryKind::RequestStakeDebit,
            )?),
        )
        .order_by_asc(entities::credit_ledger_entry::Column::Id)
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::credit_ledger_entry::Model::try_into_domain)
        .transpose()
}

pub async fn insert_request_row<C>(conn: &C, request: &Request) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::request::Model::from_domain(request)?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;
    replace_object_reference(
        conn,
        "request_snapshot",
        &request.id,
        request.git_snapshot.as_ref(),
    )
    .await?;
    Ok(())
}

pub async fn delete_request_rows<C>(conn: &C, request_id: &str) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::request_event::Entity::delete_many()
        .filter(entities::request_event::Column::RequestId.eq(request_id.to_string()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    entities::request::Entity::delete_by_id(request_id.to_string())
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    delete_object_reference(conn, "request_snapshot", request_id).await?;
    Ok(())
}

fn encode_credit_ledger_entry_kind(kind: CreditLedgerEntryKind) -> Result<String, ApiError> {
    match serde_json::to_value(kind).map_err(ApiError::internal)? {
        serde_json::Value::String(value) => Ok(value),
        _ => Err(ApiError::internal_message(
            "credit ledger entry kind did not serialize to string",
        )),
    }
}

pub async fn save_request_row<C>(conn: &C, request: &Request) -> Result<(), ApiError>
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
        .col_expr(entities::request::Column::State, Expr::value(row.state))
        .col_expr(
            entities::request::Column::ActivityVersion,
            Expr::value(row.activity_version),
        )
        .col_expr(
            entities::request::Column::StakeCredits,
            Expr::value(row.stake_credits),
        )
        .col_expr(
            entities::request::Column::Disposition,
            Expr::value(row.disposition),
        )
        .col_expr(
            entities::request::Column::Settlement,
            Expr::value(row.settlement),
        )
        .col_expr(
            entities::request::Column::UpdatedAtUnix,
            Expr::value(row.updated_at_unix),
        )
        .col_expr(
            entities::request::Column::ResolvedAtUnix,
            Expr::value(row.resolved_at_unix),
        )
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    if result.rows_affected == 0 {
        return Err(ApiError::internal_message(
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

pub async fn save_credit_account_row<C>(
    conn: &C,
    account: &UserCreditAccount,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let row = entities::user_credit_account::Model::from_domain(account)?;
    if entities::user_credit_account::Entity::find_by_id(row.user_id.clone())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .is_some()
    {
        entities::user_credit_account::Entity::update_many()
            .filter(entities::user_credit_account::Column::UserId.eq(row.user_id))
            .col_expr(
                entities::user_credit_account::Column::BalanceCredits,
                Expr::value(row.balance_credits),
            )
            .exec(conn)
            .await
            .map_err(ApiError::internal)?;
    } else {
        row.into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }
    Ok(())
}

pub async fn insert_request_event_row<C>(conn: &C, event: &RequestEvent) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::request_event::Model::from_domain(event)?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

pub async fn insert_credit_ledger_entry_row<C>(
    conn: &C,
    entry: &CreditLedgerEntry,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::credit_ledger_entry::Model::from_domain(entry)?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}
