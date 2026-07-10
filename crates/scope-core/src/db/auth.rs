use super::{
    MetadataStore, acquire_metadata_read_lock, acquire_metadata_write_lock,
    cli_sessions::create_cli_session_token_in_tx, entities,
};
use crate::{
    auth::{
        cli as cli_auth_rules,
        device::{DeviceLoginPoll, DeviceLoginStart, random_prefixed_token, random_user_code},
        tokens::token_hash,
    },
    config::{
        CLI_DEVICE_CODE_PREFIX, CLI_DEVICE_LOGIN_POLL_INTERVAL_SECS, CLI_DEVICE_LOGIN_TTL_SECS,
    },
    domain::store::UserAccount,
    error::ApiError,
    persistence::unix_now,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    TransactionTrait, sea_query::Expr,
};
use std::sync::Arc;

impl MetadataStore {
    pub async fn start_cli_device_login(
        &self,
        app_origin: &str,
    ) -> Result<DeviceLoginStart, ApiError> {
        let app_origin = app_origin.trim_end_matches('/').to_string();
        let now = unix_now()?;
        let expires_at_unix = now + CLI_DEVICE_LOGIN_TTL_SECS;

        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_metadata_write_lock(&tx).await?;
        cleanup_expired_cli_rows(&tx, now).await?;
        enforce_device_login_start_limits(&tx, now).await?;

        let (device_code, user_code) = loop {
            let device_code = random_prefixed_token(CLI_DEVICE_CODE_PREFIX)?;
            let user_code = random_user_code()?;
            let device_code_hash = token_hash(&device_code);
            let user_code_hash = token_hash(&normalize_user_code(&user_code));
            let device_exists = entities::cli_device_login::Entity::find_by_id(device_code_hash)
                .one(&tx)
                .await
                .map_err(ApiError::internal)?
                .is_some();
            let user_exists = entities::cli_device_login::Entity::find()
                .filter(entities::cli_device_login::Column::UserCodeHash.eq(user_code_hash))
                .one(&tx)
                .await
                .map_err(ApiError::internal)?
                .is_some();
            if !device_exists && !user_exists {
                break (device_code, user_code);
            }
        };

        entities::cli_device_login::Model {
            device_code_hash: token_hash(&device_code),
            user_code_hash: token_hash(&normalize_user_code(&user_code)),
            created_at_unix: u64_to_i64(now)?,
            expires_at_unix: u64_to_i64(expires_at_unix)?,
            completed_user_id: None,
            completed_at_unix: None,
            consumed_at_unix: None,
        }
        .into_active_model()
        .insert(&tx)
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;

        Ok(DeviceLoginStart {
            device_code,
            user_code,
            verification_url: format!("{app_origin}/cli-login"),
            expires_at_unix,
            poll_interval_secs: CLI_DEVICE_LOGIN_POLL_INTERVAL_SECS,
        })
    }

    pub async fn complete_cli_device_login(
        &self,
        raw_user_code: &str,
        user: &UserAccount,
    ) -> Result<(), ApiError> {
        let user_code_hash = token_hash(&normalize_user_code(raw_user_code));
        let user_id = user.id.clone();
        let now = unix_now()?;
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_metadata_write_lock(&tx).await?;

        let Some(login) = entities::cli_device_login::Entity::find()
            .filter(entities::cli_device_login::Column::UserCodeHash.eq(user_code_hash))
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
        else {
            return Err(ApiError::not_found("CLI login code not found"));
        };

        match cli_auth_rules::decide_device_login_completion(
            cli_auth_rules::DeviceLoginCompletionState {
                expires_at_unix: i64_to_u64(login.expires_at_unix)?,
                completed: login.completed_user_id.is_some(),
            },
            now,
        )? {
            cli_auth_rules::DeviceLoginCompletionDecision::Expired => {
                entities::cli_device_login::Entity::delete_by_id(login.device_code_hash)
                    .exec(&tx)
                    .await
                    .map_err(ApiError::internal)?;
                return Err(ApiError::conflict("CLI login code expired"));
            }
            cli_auth_rules::DeviceLoginCompletionDecision::Complete => {}
        }

        cleanup_expired_cli_rows(&tx, now).await?;
        entities::cli_device_login::Entity::update_many()
            .filter(entities::cli_device_login::Column::DeviceCodeHash.eq(login.device_code_hash))
            .col_expr(
                entities::cli_device_login::Column::CompletedUserId,
                Expr::value(user_id),
            )
            .col_expr(
                entities::cli_device_login::Column::CompletedAtUnix,
                Expr::value(u64_to_i64(now)?),
            )
            .exec(&tx)
            .await
            .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(())
    }

    pub async fn poll_cli_device_login(
        &self,
        device_code: &str,
    ) -> Result<DeviceLoginPoll, ApiError> {
        let device_code_hash = token_hash(device_code);
        let now = unix_now()?;
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_metadata_write_lock(&tx).await?;

        let Some(login) = entities::cli_device_login::Entity::find_by_id(device_code_hash)
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
        else {
            return Err(ApiError::not_found("CLI device login not found"));
        };
        match cli_auth_rules::decide_device_login_poll(
            cli_auth_rules::DeviceLoginPollState {
                expires_at_unix: i64_to_u64(login.expires_at_unix)?,
                consumed: login.consumed_at_unix.is_some(),
                completed_user_id: login.completed_user_id.clone(),
            },
            now,
        )? {
            cli_auth_rules::DeviceLoginPollDecision::Expired => {
                entities::cli_device_login::Entity::delete_by_id(login.device_code_hash)
                    .exec(&tx)
                    .await
                    .map_err(ApiError::internal)?;
                Err(ApiError::conflict("CLI device login expired"))
            }
            cli_auth_rules::DeviceLoginPollDecision::Pending { expires_at_unix } => {
                tx.commit().await.map_err(ApiError::internal)?;
                Ok(DeviceLoginPoll::Pending { expires_at_unix })
            }
            cli_auth_rules::DeviceLoginPollDecision::Complete { user_id } => {
                cleanup_expired_cli_rows(&tx, now).await?;
                let token = create_cli_session_token_in_tx(&tx, &user_id, now).await?;
                entities::cli_device_login::Entity::update_many()
                    .filter(
                        entities::cli_device_login::Column::DeviceCodeHash
                            .eq(login.device_code_hash),
                    )
                    .col_expr(
                        entities::cli_device_login::Column::ConsumedAtUnix,
                        Expr::value(u64_to_i64(now)?),
                    )
                    .exec(&tx)
                    .await
                    .map_err(ApiError::internal)?;
                tx.commit().await.map_err(ApiError::internal)?;
                Ok(DeviceLoginPoll::Complete {
                    session_token: token.session_token,
                    expires_at_unix: token.expires_at_unix,
                    identity: token.identity,
                })
            }
        }
    }

    pub async fn verify_cli_session_token(
        &self,
        session_token: &str,
    ) -> Result<UserAccount, ApiError> {
        let token_hash = token_hash(session_token);
        let now = unix_now()?;
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_metadata_read_lock(&tx).await?;
        let Some(session) = entities::cli_session::Entity::find()
            .filter(entities::cli_session::Column::TokenHash.eq(token_hash))
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
        else {
            return Err(ApiError::unauthorized("invalid CLI token"));
        };
        let user_id = match cli_auth_rules::decide_cli_session_use(
            cli_auth_rules::CliSessionState {
                expires_at_unix: i64_to_u64(session.expires_at_unix)?,
                revoked: session.revoked_at_unix.is_some(),
                user_id: session.user_id.clone(),
            },
            now,
        )? {
            cli_auth_rules::CliSessionUseDecision::Expired => {
                return Err(ApiError::unauthorized("CLI token expired"));
            }
            cli_auth_rules::CliSessionUseDecision::Active { user_id } => user_id,
        };
        let user = load_user_by_id(&tx, &user_id).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(user)
    }

    pub async fn revoke_cli_session_token(&self, session_token: &str) -> Result<(), ApiError> {
        let token_hash = token_hash(session_token);
        let now = unix_now()?;
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_metadata_write_lock(&tx).await?;
        let Some(session) = entities::cli_session::Entity::find()
            .filter(entities::cli_session::Column::TokenHash.eq(token_hash))
            .one(&tx)
            .await
            .map_err(ApiError::internal)?
        else {
            return Err(ApiError::unauthorized("invalid CLI token"));
        };
        match cli_auth_rules::decide_cli_session_revoke(i64_to_u64(session.expires_at_unix)?, now) {
            cli_auth_rules::CliSessionRevokeDecision::Expired => {
                entities::cli_session::Entity::delete_by_id(session.id)
                    .exec(&tx)
                    .await
                    .map_err(ApiError::internal)?;
                return Err(ApiError::unauthorized("CLI token expired"));
            }
            cli_auth_rules::CliSessionRevokeDecision::Revoke => {}
        }
        entities::cli_session::Entity::update_many()
            .filter(entities::cli_session::Column::Id.eq(session.id))
            .col_expr(
                entities::cli_session::Column::RevokedAtUnix,
                Expr::value(u64_to_i64(now)?),
            )
            .exec(&tx)
            .await
            .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(())
    }
}

pub async fn cleanup_expired_cli_rows<C>(conn: &C, now: u64) -> Result<(), ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let now = u64_to_i64(now)?;
    entities::cli_device_login::Entity::delete_many()
        .filter(entities::cli_device_login::Column::ExpiresAtUnix.lte(now))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    entities::cli_browser_login::Entity::delete_many()
        .filter(entities::cli_browser_login::Column::ExpiresAtUnix.lte(now))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    entities::cli_exchange_grant::Entity::delete_many()
        .filter(entities::cli_exchange_grant::Column::ExpiresAtUnix.lte(now))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    entities::cli_session::Entity::delete_many()
        .filter(entities::cli_session::Column::ExpiresAtUnix.lte(now))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

async fn enforce_device_login_start_limits<C>(conn: &C, now: u64) -> Result<(), ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let pending_count = entities::cli_device_login::Entity::find()
        .count(conn)
        .await
        .map_err(ApiError::internal)?;
    let window_start = u64_to_i64(cli_auth_rules::device_login_start_window_start(now))?;
    let window_count = entities::cli_device_login::Entity::find()
        .filter(entities::cli_device_login::Column::CreatedAtUnix.gte(window_start))
        .count(conn)
        .await
        .map_err(ApiError::internal)?;
    cli_auth_rules::enforce_device_login_start_rate_limit(pending_count, window_count)
}

pub async fn load_user_by_id<C>(conn: &C, user_id: &str) -> Result<UserAccount, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    entities::user::Entity::find_by_id(user_id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal_message("signed-in user was not persisted"))?
        .try_into_domain()
}

fn normalize_user_code(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-')
        .flat_map(char::to_uppercase)
        .collect()
}

pub fn u64_to_i64(value: u64) -> Result<i64, ApiError> {
    i64::try_from(value).map_err(ApiError::internal)
}

pub fn i64_to_u64(value: i64) -> Result<u64, ApiError> {
    u64::try_from(value).map_err(ApiError::internal)
}
