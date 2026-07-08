use super::entities;
use crate::{
    domain::requests::{
        CreditLedgerEntry, CreditLedgerEntryKind, Request, RequestEvent, UserCreditAccount,
    },
    error::ApiError,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, sea_query::Expr,
};
use std::collections::BTreeMap;

pub struct RequestCatalogRows {
    pub requests: BTreeMap<String, Request>,
    pub request_events: BTreeMap<String, RequestEvent>,
    pub user_credit_accounts: BTreeMap<String, UserCreditAccount>,
    pub credit_ledger_entries: BTreeMap<String, CreditLedgerEntry>,
}

pub async fn load_request_catalog_rows<C>(conn: &C) -> Result<RequestCatalogRows, ApiError>
where
    C: ConnectionTrait,
{
    let requests = entities::request::Entity::find()
        .order_by_asc(entities::request::Column::Id)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|row| {
            let request = row.try_into_domain()?;
            Ok((request.id.clone(), request))
        })
        .collect::<Result<BTreeMap<_, _>, ApiError>>()?;

    let request_events = entities::request_event::Entity::find()
        .order_by_asc(entities::request_event::Column::RequestId)
        .order_by_asc(entities::request_event::Column::CreatedAtUnix)
        .order_by_asc(entities::request_event::Column::Id)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|row| {
            let event = row.try_into_domain()?;
            Ok((event.id.clone(), event))
        })
        .collect::<Result<BTreeMap<_, _>, ApiError>>()?;

    let user_credit_accounts = entities::user_credit_account::Entity::find()
        .order_by_asc(entities::user_credit_account::Column::UserId)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|row| {
            let account = row.into_domain();
            (account.user_id.clone(), account)
        })
        .collect();

    let credit_ledger_entries = entities::credit_ledger_entry::Entity::find()
        .order_by_asc(entities::credit_ledger_entry::Column::UserId)
        .order_by_asc(entities::credit_ledger_entry::Column::CreatedAtUnix)
        .order_by_asc(entities::credit_ledger_entry::Column::Id)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|row| {
            let entry = row.try_into_domain()?;
            Ok((entry.id.clone(), entry))
        })
        .collect::<Result<BTreeMap<_, _>, ApiError>>()?;

    Ok(RequestCatalogRows {
        requests,
        request_events,
        user_credit_accounts,
        credit_ledger_entries,
    })
}

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

pub async fn request_by_ref<C>(conn: &C, request_ref: &str) -> Result<Option<Request>, ApiError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find()
        .filter(entities::request::Column::RequestRef.eq(request_ref.to_string()))
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
        .order_by_asc(entities::request_event::Column::Id)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(entities::request_event::Model::try_into_domain)
        .collect()
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
    Ok(
        entities::user_credit_account::Entity::find_by_id(user_id.to_string())
            .one(conn)
            .await
            .map_err(ApiError::internal)?
            .map(entities::user_credit_account::Model::into_domain),
    )
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
    Ok(())
}

pub async fn save_credit_account_row<C>(
    conn: &C,
    account: &UserCreditAccount,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let row = entities::user_credit_account::Model::from_domain(account);
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
