use super::{
    AuthStore, acquire_aggregate_lock,
    cli_auth_results::{DeviceLoginPoll, NewCliSession, StartDeviceLoginCommand},
    cli_sessions::insert_cli_session_in_tx,
    entities,
};
use crate::error::PostgresError;
use scope_domain::{account::cli_auth as cli_auth_rules, store::UserAccount};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    TransactionTrait, sea_query::Expr,
};
use std::sync::Arc;

impl AuthStore {
    pub async fn start_cli_device_login(
        &self,
        command: StartDeviceLoginCommand,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-auth", "start").await?;
        cleanup_expired_cli_rows(&tx, now_unix).await?;
        enforce_device_login_start_limits(&tx, now_unix).await?;
        entities::cli_device_login::Model {
            device_code_hash: command.device_code_hash,
            user_code_hash: command.user_code_hash,
            created_at_unix: u64_to_i64(command.created_at_unix)?,
            expires_at_unix: u64_to_i64(command.expires_at_unix)?,
            completed_user_id: None,
            completed_at_unix: None,
            consumed_at_unix: None,
        }
        .into_active_model()
        .insert(&tx)
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn complete_cli_device_login_by_user_code_hash(
        &self,
        user_code_hash: &str,
        user: &UserAccount,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let user_code_hash = user_code_hash.to_string();
        let user_id = user.id.clone();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-device-user-code", &user_code_hash).await?;

        let Some(login) = entities::cli_device_login::Entity::find()
            .filter(entities::cli_device_login::Column::UserCodeHash.eq(user_code_hash))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            return Err(PostgresError::not_found("CLI login code not found"));
        };

        match cli_auth_rules::decide_device_login_completion(
            cli_auth_rules::DeviceLoginCompletionState {
                expires_at_unix: i64_to_u64(login.expires_at_unix)?,
                completed: login.completed_user_id.is_some(),
            },
            now_unix,
        )? {
            cli_auth_rules::DeviceLoginCompletionDecision::Expired => {
                entities::cli_device_login::Entity::delete_by_id(login.device_code_hash)
                    .exec(&tx)
                    .await
                    .map_err(PostgresError::internal)?;
                return Err(PostgresError::conflict("CLI login code expired"));
            }
            cli_auth_rules::DeviceLoginCompletionDecision::Complete => {}
        }

        cleanup_expired_cli_rows(&tx, now_unix).await?;
        entities::cli_device_login::Entity::update_many()
            .filter(entities::cli_device_login::Column::DeviceCodeHash.eq(login.device_code_hash))
            .col_expr(
                entities::cli_device_login::Column::CompletedUserId,
                Expr::value(user_id),
            )
            .col_expr(
                entities::cli_device_login::Column::CompletedAtUnix,
                Expr::value(u64_to_i64(now_unix)?),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn poll_cli_device_login_by_hash(
        &self,
        device_code_hash: &str,
        session: NewCliSession,
        now_unix: u64,
    ) -> Result<DeviceLoginPoll, PostgresError> {
        let device_code_hash = device_code_hash.to_string();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-device-code", &device_code_hash).await?;

        let Some(login) = entities::cli_device_login::Entity::find_by_id(device_code_hash)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            return Err(PostgresError::not_found("CLI device login not found"));
        };
        match cli_auth_rules::decide_device_login_poll(
            cli_auth_rules::DeviceLoginPollState {
                expires_at_unix: i64_to_u64(login.expires_at_unix)?,
                consumed: login.consumed_at_unix.is_some(),
                completed_user_id: login.completed_user_id.clone(),
            },
            now_unix,
        )? {
            cli_auth_rules::DeviceLoginPollDecision::Expired => {
                entities::cli_device_login::Entity::delete_by_id(login.device_code_hash)
                    .exec(&tx)
                    .await
                    .map_err(PostgresError::internal)?;
                Err(PostgresError::conflict("CLI device login expired"))
            }
            cli_auth_rules::DeviceLoginPollDecision::Pending { expires_at_unix } => {
                tx.commit().await.map_err(PostgresError::internal)?;
                Ok(DeviceLoginPoll::Pending { expires_at_unix })
            }
            cli_auth_rules::DeviceLoginPollDecision::Complete { user_id } => {
                cleanup_expired_cli_rows(&tx, now_unix).await?;
                let identity = insert_cli_session_in_tx(&tx, &user_id, session).await?;
                entities::cli_device_login::Entity::update_many()
                    .filter(
                        entities::cli_device_login::Column::DeviceCodeHash
                            .eq(login.device_code_hash),
                    )
                    .col_expr(
                        entities::cli_device_login::Column::ConsumedAtUnix,
                        Expr::value(u64_to_i64(now_unix)?),
                    )
                    .exec(&tx)
                    .await
                    .map_err(PostgresError::internal)?;
                tx.commit().await.map_err(PostgresError::internal)?;
                Ok(DeviceLoginPoll::Complete { identity })
            }
        }
    }

    pub async fn verify_cli_session_by_hash(
        &self,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<UserAccount, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let Some(session) = entities::cli_session::Entity::find()
            .filter(entities::cli_session::Column::TokenHash.eq(token_hash))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            return Err(PostgresError::unauthenticated("invalid CLI token"));
        };
        let user_id = match cli_auth_rules::decide_cli_session_use(
            cli_auth_rules::CliSessionState {
                expires_at_unix: i64_to_u64(session.expires_at_unix)?,
                revoked: session.revoked_at_unix.is_some(),
                user_id: session.user_id.clone(),
            },
            now_unix,
        )? {
            cli_auth_rules::CliSessionUseDecision::Expired => {
                return Err(PostgresError::unauthenticated("CLI token expired"));
            }
            cli_auth_rules::CliSessionUseDecision::Active { user_id } => user_id,
        };
        let user = load_user_by_id(&tx, &user_id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(user)
    }

    pub async fn revoke_cli_session_by_hash(
        &self,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let token_hash = token_hash.to_string();
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-session-token", &token_hash).await?;
        let Some(session) = entities::cli_session::Entity::find()
            .filter(entities::cli_session::Column::TokenHash.eq(token_hash))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            return Err(PostgresError::unauthenticated("invalid CLI token"));
        };
        match cli_auth_rules::decide_cli_session_revoke(
            i64_to_u64(session.expires_at_unix)?,
            now_unix,
        ) {
            cli_auth_rules::CliSessionRevokeDecision::Expired => {
                entities::cli_session::Entity::delete_by_id(session.id)
                    .exec(&tx)
                    .await
                    .map_err(PostgresError::internal)?;
                return Err(PostgresError::unauthenticated("CLI token expired"));
            }
            cli_auth_rules::CliSessionRevokeDecision::Revoke => {}
        }
        entities::cli_session::Entity::update_many()
            .filter(entities::cli_session::Column::Id.eq(session.id))
            .col_expr(
                entities::cli_session::Column::RevokedAtUnix,
                Expr::value(u64_to_i64(now_unix)?),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(())
    }
}

pub async fn cleanup_expired_cli_rows<C>(conn: &C, now_unix: u64) -> Result<(), PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let now_unix = u64_to_i64(now_unix)?;
    entities::cli_device_login::Entity::delete_many()
        .filter(entities::cli_device_login::Column::ExpiresAtUnix.lte(now_unix))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    entities::cli_browser_login::Entity::delete_many()
        .filter(entities::cli_browser_login::Column::ExpiresAtUnix.lte(now_unix))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    entities::cli_exchange_grant::Entity::delete_many()
        .filter(entities::cli_exchange_grant::Column::ExpiresAtUnix.lte(now_unix))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    entities::cli_session::Entity::delete_many()
        .filter(entities::cli_session::Column::ExpiresAtUnix.lte(now_unix))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    Ok(())
}

async fn enforce_device_login_start_limits<C>(conn: &C, now_unix: u64) -> Result<(), PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let pending_count = entities::cli_device_login::Entity::find()
        .count(conn)
        .await
        .map_err(PostgresError::internal)?;
    let window_start = u64_to_i64(cli_auth_rules::device_login_start_window_start(now_unix))?;
    let window_count = entities::cli_device_login::Entity::find()
        .filter(entities::cli_device_login::Column::CreatedAtUnix.gte(window_start))
        .count(conn)
        .await
        .map_err(PostgresError::internal)?;
    Ok(cli_auth_rules::enforce_device_login_start_rate_limit(
        pending_count,
        window_count,
    )?)
}

pub async fn load_user_by_id<C>(conn: &C, user_id: &str) -> Result<UserAccount, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    entities::user::Entity::find_by_id(user_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("signed-in user was not persisted"))?
        .try_into_domain()
}

pub fn u64_to_i64(value: u64) -> Result<i64, PostgresError> {
    i64::try_from(value).map_err(PostgresError::internal)
}

pub fn i64_to_u64(value: i64) -> Result<u64, PostgresError> {
    u64::try_from(value).map_err(PostgresError::internal)
}
