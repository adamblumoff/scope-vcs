use super::cli_sessions::{cli_session_summary_from_model, insert_cli_session_in_tx};
use super::{
    AuthStore, acquire_aggregate_lock,
    auth::{cleanup_expired_cli_rows, i64_to_u64, u64_to_i64},
    cli_auth_results::{
        BrowserLoginCompletion, CliSessionSummary, CreateCliExchangeGrantCommand, NewCliSession,
        StartBrowserLoginCommand,
    },
    entities,
};
use crate::error::PostgresError;
use scope_domain::{
    account::SessionIdentity, account::cli_auth as cli_auth_rules, store::UserAccount,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    QueryOrder, TransactionTrait, sea_query::Expr,
};
use std::sync::Arc;

impl AuthStore {
    pub async fn start_cli_browser_login(
        &self,
        command: StartBrowserLoginCommand,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let db = Arc::clone(&self.db);
        let row = entities::cli_browser_login::Model {
            request_id: command.request_id,
            request_secret_hash: command.request_secret_hash,
            callback_url: command.callback_url,
            callback_code_hash: None,
            created_at_unix: u64_to_i64(command.created_at_unix)?,
            expires_at_unix: u64_to_i64(command.expires_at_unix)?,
            completed_user_id: None,
            completed_at_unix: None,
            consumed_at_unix: None,
        };
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-auth", "start").await?;
        cleanup_expired_cli_rows(&tx, now_unix).await?;
        enforce_browser_login_start_limits(&tx, now_unix).await?;
        row.into_active_model()
            .insert(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn complete_cli_browser_login(
        &self,
        request_id: &str,
        callback_code_hash: String,
        user: &UserAccount,
        now_unix: u64,
    ) -> Result<BrowserLoginCompletion, PostgresError> {
        let db = Arc::clone(&self.db);
        let request_id = request_id.to_string();
        let user_id = user.id.clone();
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-browser-request", &request_id).await?;
        let Some(login) = entities::cli_browser_login::Entity::find_by_id(request_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            return Err(PostgresError::not_found("CLI browser login not found"));
        };
        match cli_auth_rules::decide_browser_login_completion(
            cli_auth_rules::BrowserLoginCompletionState {
                expires_at_unix: i64_to_u64(login.expires_at_unix)?,
                consumed: login.consumed_at_unix.is_some(),
                completed: login.completed_user_id.is_some() || login.callback_code_hash.is_some(),
            },
            now_unix,
        )? {
            cli_auth_rules::BrowserLoginCompletionDecision::Expired => {
                entities::cli_browser_login::Entity::delete_by_id(login.request_id)
                    .exec(&tx)
                    .await
                    .map_err(PostgresError::internal)?;
                return Err(PostgresError::conflict("CLI browser login expired"));
            }
            cli_auth_rules::BrowserLoginCompletionDecision::Complete => {}
        }

        entities::cli_browser_login::Entity::update_many()
            .filter(entities::cli_browser_login::Column::RequestId.eq(request_id.clone()))
            .col_expr(
                entities::cli_browser_login::Column::CallbackCodeHash,
                Expr::value(callback_code_hash),
            )
            .col_expr(
                entities::cli_browser_login::Column::CompletedUserId,
                Expr::value(user_id),
            )
            .col_expr(
                entities::cli_browser_login::Column::CompletedAtUnix,
                Expr::value(u64_to_i64(now_unix)?),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(BrowserLoginCompletion {
            request_id,
            callback_url: login.callback_url,
        })
    }

    pub async fn exchange_cli_browser_login(
        &self,
        request_id: &str,
        request_secret_hash: &str,
        callback_code_hash: &str,
        session: NewCliSession,
        now_unix: u64,
    ) -> Result<SessionIdentity, PostgresError> {
        let db = Arc::clone(&self.db);
        let request_id = request_id.to_string();
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-browser-request", &request_id).await?;
        let Some(login) = entities::cli_browser_login::Entity::find_by_id(request_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            return Err(PostgresError::not_found("CLI browser login not found"));
        };
        let user_id = match cli_auth_rules::decide_browser_login_exchange(
            cli_auth_rules::BrowserLoginExchangeState {
                expires_at_unix: i64_to_u64(login.expires_at_unix)?,
                consumed: login.consumed_at_unix.is_some(),
                request_secret_hash: login.request_secret_hash.clone(),
                callback_code_hash: login.callback_code_hash.clone(),
                completed_user_id: login.completed_user_id.clone(),
            },
            now_unix,
            request_secret_hash,
            callback_code_hash,
        )? {
            cli_auth_rules::BrowserLoginExchangeDecision::Expired => {
                entities::cli_browser_login::Entity::delete_by_id(login.request_id)
                    .exec(&tx)
                    .await
                    .map_err(PostgresError::internal)?;
                return Err(PostgresError::conflict("CLI browser login expired"));
            }
            cli_auth_rules::BrowserLoginExchangeDecision::Complete { user_id } => user_id,
        };

        entities::cli_browser_login::Entity::update_many()
            .filter(entities::cli_browser_login::Column::RequestId.eq(request_id))
            .col_expr(
                entities::cli_browser_login::Column::ConsumedAtUnix,
                Expr::value(u64_to_i64(now_unix)?),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let identity = insert_cli_session_in_tx(&tx, &user_id, session).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(identity)
    }

    pub async fn create_cli_exchange_grant(
        &self,
        command: CreateCliExchangeGrantCommand,
        user: &UserAccount,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let db = Arc::clone(&self.db);
        let row = entities::cli_exchange_grant::Model {
            grant_hash: command.grant_hash,
            user_id: user.id.clone(),
            created_at_unix: u64_to_i64(command.created_at_unix)?,
            expires_at_unix: u64_to_i64(command.expires_at_unix)?,
            consumed_at_unix: None,
        };
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-exchange-grant", &row.grant_hash).await?;
        cleanup_expired_cli_rows(&tx, now_unix).await?;
        row.into_active_model()
            .insert(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn exchange_cli_grant_by_hash(
        &self,
        grant_hash: &str,
        session: NewCliSession,
        now_unix: u64,
    ) -> Result<SessionIdentity, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-exchange-grant", grant_hash).await?;
        let Some(grant) = entities::cli_exchange_grant::Entity::find_by_id(grant_hash)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            return Err(PostgresError::unauthenticated("invalid CLI exchange token"));
        };
        let user_id = match cli_auth_rules::decide_cli_exchange_grant(
            cli_auth_rules::CliExchangeGrantState {
                expires_at_unix: i64_to_u64(grant.expires_at_unix)?,
                consumed: grant.consumed_at_unix.is_some(),
                user_id: grant.user_id.clone(),
            },
            now_unix,
        )? {
            cli_auth_rules::CliExchangeGrantDecision::Expired => {
                entities::cli_exchange_grant::Entity::delete_by_id(grant.grant_hash)
                    .exec(&tx)
                    .await
                    .map_err(PostgresError::internal)?;
                return Err(PostgresError::conflict("CLI exchange token expired"));
            }
            cli_auth_rules::CliExchangeGrantDecision::Complete { user_id } => user_id,
        };

        entities::cli_exchange_grant::Entity::update_many()
            .filter(entities::cli_exchange_grant::Column::GrantHash.eq(grant.grant_hash.clone()))
            .col_expr(
                entities::cli_exchange_grant::Column::ConsumedAtUnix,
                Expr::value(u64_to_i64(now_unix)?),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let identity = insert_cli_session_in_tx(&tx, &user_id, session).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(identity)
    }

    pub async fn list_cli_sessions_for_user(
        &self,
        user: &UserAccount,
        now_unix: u64,
    ) -> Result<Vec<CliSessionSummary>, PostgresError> {
        let user_id = user.id.clone();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let sessions = entities::cli_session::Entity::find()
            .filter(entities::cli_session::Column::UserId.eq(user_id))
            .filter(entities::cli_session::Column::RevokedAtUnix.is_null())
            .filter(entities::cli_session::Column::ExpiresAtUnix.gt(u64_to_i64(now_unix)?))
            .order_by_desc(entities::cli_session::Column::CreatedAtUnix)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(cli_session_summary_from_model)
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(sessions)
    }

    pub async fn revoke_cli_session_for_user(
        &self,
        user: &UserAccount,
        session_id: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let user_id = user.id.clone();
        let session_id = session_id.to_string();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-session", &session_id).await?;
        cleanup_expired_cli_rows(&tx, now_unix).await?;
        let result = entities::cli_session::Entity::update_many()
            .filter(entities::cli_session::Column::Id.eq(session_id))
            .filter(entities::cli_session::Column::UserId.eq(user_id))
            .filter(entities::cli_session::Column::RevokedAtUnix.is_null())
            .col_expr(
                entities::cli_session::Column::RevokedAtUnix,
                Expr::value(u64_to_i64(now_unix)?),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        if result.rows_affected == 0 {
            return Err(PostgresError::not_found("CLI session not found"));
        }
        Ok(())
    }
}

async fn enforce_browser_login_start_limits<C>(conn: &C, now_unix: u64) -> Result<(), PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let pending_count = entities::cli_browser_login::Entity::find()
        .count(conn)
        .await
        .map_err(PostgresError::internal)?;
    let window_start = u64_to_i64(cli_auth_rules::browser_login_start_window_start(now_unix))?;
    let window_count = entities::cli_browser_login::Entity::find()
        .filter(entities::cli_browser_login::Column::CreatedAtUnix.gte(window_start))
        .count(conn)
        .await
        .map_err(PostgresError::internal)?;
    Ok(cli_auth_rules::enforce_browser_login_start_rate_limit(
        pending_count,
        window_count,
    )?)
}
