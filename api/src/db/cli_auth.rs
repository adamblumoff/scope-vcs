#[cfg(test)]
use super::auth::{MemoryBrowserLogin, MemoryCliSession, MemoryExchangeGrant};
use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock,
    auth::{cleanup_expired_cli_rows, i64_to_u64, load_user_by_id, u64_to_i64},
    entities, run_api_db_on,
};
use crate::{
    auth::{
        device::{
            BROWSER_LOGIN_START_WINDOW_SECS, BrowserLoginStart, CliExchangeGrant,
            CliSessionSummary, CliSessionToken, MAX_BROWSER_LOGIN_STARTS_PER_WINDOW,
            MAX_PENDING_BROWSER_LOGINS, random_prefixed_token,
        },
        tokens::token_hash,
    },
    config::{
        CLI_BROWSER_LOGIN_ID_PREFIX, CLI_BROWSER_LOGIN_SECRET_PREFIX, CLI_BROWSER_LOGIN_TTL_SECS,
        CLI_CALLBACK_CODE_PREFIX, CLI_EXCHANGE_GRANT_PREFIX, CLI_EXCHANGE_GRANT_TTL_SECS,
        CLI_SESSION_ID_PREFIX, CLI_SESSION_TOKEN_PREFIX, CLI_SESSION_TTL_SECS,
    },
    domain::store::UserAccount,
    error::ApiError,
    http::responses::SessionIdentity,
    persistence::unix_now,
};
use reqwest::Url;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    QueryOrder, TransactionTrait, sea_query::Expr,
};
use std::sync::Arc;

impl MetadataStore {
    pub(crate) fn start_cli_browser_login(
        &self,
        app_origin: &str,
        callback_url: &str,
    ) -> Result<BrowserLoginStart, ApiError> {
        validate_loopback_callback_url(callback_url)?;
        let now = unix_now()?;
        let request_id = random_prefixed_token(CLI_BROWSER_LOGIN_ID_PREFIX)?;
        let request_secret = random_prefixed_token(CLI_BROWSER_LOGIN_SECRET_PREFIX)?;
        let expires_at_unix = now + CLI_BROWSER_LOGIN_TTL_SECS;
        let authorization_url = browser_authorization_url(app_origin, &request_id)?;

        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                let row = entities::cli_browser_login::Model {
                    request_id: request_id.clone(),
                    request_secret_hash: token_hash(&request_secret),
                    callback_url: callback_url.to_string(),
                    callback_code_hash: None,
                    created_at_unix: u64_to_i64(now)?,
                    expires_at_unix: u64_to_i64(expires_at_unix)?,
                    completed_user_id: None,
                    completed_at_unix: None,
                    consumed_at_unix: None,
                };
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    cleanup_expired_cli_rows(&tx, now).await?;
                    enforce_browser_login_start_limits(&tx, now).await?;
                    row.into_active_model()
                        .insert(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(())
                })?;
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                auth.cleanup_expired(now);
                enforce_memory_browser_login_start_limits(&auth, now)?;
                auth.cli_browser_logins.insert(
                    request_id.clone(),
                    MemoryBrowserLogin {
                        request_secret_hash: token_hash(&request_secret),
                        callback_url: callback_url.to_string(),
                        callback_code_hash: None,
                        created_at_unix: now,
                        expires_at_unix,
                        completed_user_id: None,
                        completed_at_unix: None,
                        consumed_at_unix: None,
                    },
                );
            }
        }

        Ok(BrowserLoginStart {
            request_id,
            request_secret,
            authorization_url,
            expires_at_unix,
        })
    }

    pub(crate) fn complete_cli_browser_login(
        &self,
        request_id: &str,
        user: &UserAccount,
    ) -> Result<String, ApiError> {
        let now = unix_now()?;
        let callback_code = random_prefixed_token(CLI_CALLBACK_CODE_PREFIX)?;
        let callback_code_hash = token_hash(&callback_code);

        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                let request_id = request_id.to_string();
                let user_id = user.id.clone();
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let Some(login) =
                        entities::cli_browser_login::Entity::find_by_id(request_id.clone())
                            .one(&tx)
                            .await
                            .map_err(ApiError::internal)?
                    else {
                        return Err(ApiError::not_found("CLI browser login not found"));
                    };
                    if now >= i64_to_u64(login.expires_at_unix)? {
                        entities::cli_browser_login::Entity::delete_by_id(login.request_id)
                            .exec(&tx)
                            .await
                            .map_err(ApiError::internal)?;
                        return Err(ApiError::conflict("CLI browser login expired"));
                    }
                    if login.consumed_at_unix.is_some() {
                        return Err(ApiError::conflict("CLI browser login already consumed"));
                    }
                    if login.completed_user_id.is_some() || login.callback_code_hash.is_some() {
                        return Err(ApiError::conflict("CLI browser login already completed"));
                    }

                    entities::cli_browser_login::Entity::update_many()
                        .filter(entities::cli_browser_login::Column::RequestId.eq(request_id))
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
                            Expr::value(u64_to_i64(now)?),
                        )
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    browser_callback_url(&login.callback_url, &login.request_id, &callback_code)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                let Some(login) = auth.cli_browser_logins.get_mut(request_id) else {
                    return Err(ApiError::not_found("CLI browser login not found"));
                };
                if now >= login.expires_at_unix {
                    auth.cli_browser_logins.remove(request_id);
                    return Err(ApiError::conflict("CLI browser login expired"));
                }
                if login.consumed_at_unix.is_some() {
                    return Err(ApiError::conflict("CLI browser login already consumed"));
                }
                if login.completed_user_id.is_some() || login.callback_code_hash.is_some() {
                    return Err(ApiError::conflict("CLI browser login already completed"));
                }
                login.callback_code_hash = Some(callback_code_hash);
                login.completed_user_id = Some(user.id.clone());
                login.completed_at_unix = Some(now);
                browser_callback_url(&login.callback_url, request_id, &callback_code)
            }
        }
    }

    pub(crate) fn exchange_cli_browser_login(
        &self,
        request_id: &str,
        request_secret: &str,
        callback_code: &str,
    ) -> Result<CliSessionToken, ApiError> {
        let request_secret_hash = token_hash(request_secret);
        let callback_code_hash = token_hash(callback_code);
        let now = unix_now()?;

        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                let request_id = request_id.to_string();
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let Some(login) =
                        entities::cli_browser_login::Entity::find_by_id(request_id.clone())
                            .one(&tx)
                            .await
                            .map_err(ApiError::internal)?
                    else {
                        return Err(ApiError::not_found("CLI browser login not found"));
                    };
                    if now >= i64_to_u64(login.expires_at_unix)? {
                        entities::cli_browser_login::Entity::delete_by_id(login.request_id)
                            .exec(&tx)
                            .await
                            .map_err(ApiError::internal)?;
                        return Err(ApiError::conflict("CLI browser login expired"));
                    }
                    if login.consumed_at_unix.is_some() {
                        return Err(ApiError::conflict("CLI browser login already consumed"));
                    }
                    if login.request_secret_hash != request_secret_hash {
                        return Err(ApiError::unauthorized("invalid CLI browser login secret"));
                    }
                    if login.callback_code_hash.as_deref() != Some(callback_code_hash.as_str()) {
                        return Err(ApiError::unauthorized("invalid CLI browser login code"));
                    }
                    let Some(user_id) = login.completed_user_id else {
                        return Err(ApiError::conflict("CLI browser login is pending"));
                    };

                    entities::cli_browser_login::Entity::update_many()
                        .filter(entities::cli_browser_login::Column::RequestId.eq(request_id))
                        .col_expr(
                            entities::cli_browser_login::Column::ConsumedAtUnix,
                            Expr::value(u64_to_i64(now)?),
                        )
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    let token = create_cli_session_token_in_tx(&tx, &user_id, now).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(token)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                let Some(login) = auth.cli_browser_logins.get_mut(request_id) else {
                    return Err(ApiError::not_found("CLI browser login not found"));
                };
                if now >= login.expires_at_unix {
                    auth.cli_browser_logins.remove(request_id);
                    return Err(ApiError::conflict("CLI browser login expired"));
                }
                if login.consumed_at_unix.is_some() {
                    return Err(ApiError::conflict("CLI browser login already consumed"));
                }
                if login.request_secret_hash != request_secret_hash {
                    return Err(ApiError::unauthorized("invalid CLI browser login secret"));
                }
                if login.callback_code_hash.as_deref() != Some(callback_code_hash.as_str()) {
                    return Err(ApiError::unauthorized("invalid CLI browser login code"));
                }
                let Some(user_id) = login.completed_user_id.clone() else {
                    return Err(ApiError::conflict("CLI browser login is pending"));
                };
                login.consumed_at_unix = Some(now);
                create_cli_session_token_in_memory(memory, auth, user_id, now)
            }
        }
    }

    pub(crate) fn create_cli_exchange_grant(
        &self,
        user: &UserAccount,
    ) -> Result<CliExchangeGrant, ApiError> {
        let now = unix_now()?;
        let exchange_token = random_prefixed_token(CLI_EXCHANGE_GRANT_PREFIX)?;
        let expires_at_unix = now + CLI_EXCHANGE_GRANT_TTL_SECS;

        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                let row = entities::cli_exchange_grant::Model {
                    grant_hash: token_hash(&exchange_token),
                    user_id: user.id.clone(),
                    created_at_unix: u64_to_i64(now)?,
                    expires_at_unix: u64_to_i64(expires_at_unix)?,
                    consumed_at_unix: None,
                };
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    cleanup_expired_cli_rows(&tx, now).await?;
                    row.into_active_model()
                        .insert(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(())
                })?;
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                auth.cleanup_expired(now);
                auth.cli_exchange_grants.insert(
                    token_hash(&exchange_token),
                    MemoryExchangeGrant {
                        user_id: user.id.clone(),
                        expires_at_unix,
                        consumed_at_unix: None,
                    },
                );
            }
        }

        Ok(CliExchangeGrant {
            exchange_token,
            expires_at_unix,
        })
    }

    pub(crate) fn exchange_cli_grant(
        &self,
        exchange_token: &str,
    ) -> Result<CliSessionToken, ApiError> {
        let grant_hash = token_hash(exchange_token);
        let now = unix_now()?;

        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let Some(grant) = entities::cli_exchange_grant::Entity::find_by_id(grant_hash)
                        .one(&tx)
                        .await
                        .map_err(ApiError::internal)?
                    else {
                        return Err(ApiError::unauthorized("invalid CLI exchange token"));
                    };
                    if now >= i64_to_u64(grant.expires_at_unix)? {
                        entities::cli_exchange_grant::Entity::delete_by_id(grant.grant_hash)
                            .exec(&tx)
                            .await
                            .map_err(ApiError::internal)?;
                        return Err(ApiError::conflict("CLI exchange token expired"));
                    }
                    if grant.consumed_at_unix.is_some() {
                        return Err(ApiError::conflict("CLI exchange token already used"));
                    }

                    entities::cli_exchange_grant::Entity::update_many()
                        .filter(
                            entities::cli_exchange_grant::Column::GrantHash
                                .eq(grant.grant_hash.clone()),
                        )
                        .col_expr(
                            entities::cli_exchange_grant::Column::ConsumedAtUnix,
                            Expr::value(u64_to_i64(now)?),
                        )
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    let token = create_cli_session_token_in_tx(&tx, &grant.user_id, now).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(token)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                let Some(grant) = auth.cli_exchange_grants.get_mut(&grant_hash) else {
                    return Err(ApiError::unauthorized("invalid CLI exchange token"));
                };
                if now >= grant.expires_at_unix {
                    auth.cli_exchange_grants.remove(&grant_hash);
                    return Err(ApiError::conflict("CLI exchange token expired"));
                }
                if grant.consumed_at_unix.is_some() {
                    return Err(ApiError::conflict("CLI exchange token already used"));
                }
                let user_id = grant.user_id.clone();
                grant.consumed_at_unix = Some(now);
                create_cli_session_token_in_memory(memory, auth, user_id, now)
            }
        }
    }

    pub(crate) fn list_cli_sessions_for_user(
        &self,
        user: &UserAccount,
    ) -> Result<Vec<CliSessionSummary>, ApiError> {
        let user_id = user.id.clone();
        let now = unix_now()?;
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    cleanup_expired_cli_rows(&tx, now).await?;
                    let sessions = entities::cli_session::Entity::find()
                        .filter(entities::cli_session::Column::UserId.eq(user_id))
                        .filter(entities::cli_session::Column::RevokedAtUnix.is_null())
                        .filter(entities::cli_session::Column::ExpiresAtUnix.gt(u64_to_i64(now)?))
                        .order_by_desc(entities::cli_session::Column::CreatedAtUnix)
                        .all(&tx)
                        .await
                        .map_err(ApiError::internal)?
                        .into_iter()
                        .map(cli_session_summary_from_model)
                        .collect::<Result<Vec<_>, _>>()?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(sessions)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                auth.cleanup_expired(now);
                let mut sessions = auth
                    .cli_sessions
                    .values()
                    .filter(|session| {
                        session.user_id == user_id
                            && session.revoked_at_unix.is_none()
                            && now < session.expires_at_unix
                    })
                    .map(cli_session_summary_from_memory)
                    .collect::<Vec<_>>();
                sessions.sort_by(|left, right| right.created_at_unix.cmp(&left.created_at_unix));
                Ok(sessions)
            }
        }
    }

    pub(crate) fn revoke_cli_session_for_user(
        &self,
        user: &UserAccount,
        session_id: &str,
    ) -> Result<(), ApiError> {
        let user_id = user.id.clone();
        let session_id = session_id.to_string();
        let now = unix_now()?;
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    cleanup_expired_cli_rows(&tx, now).await?;
                    let result = entities::cli_session::Entity::update_many()
                        .filter(entities::cli_session::Column::Id.eq(session_id))
                        .filter(entities::cli_session::Column::UserId.eq(user_id))
                        .filter(entities::cli_session::Column::RevokedAtUnix.is_null())
                        .col_expr(
                            entities::cli_session::Column::RevokedAtUnix,
                            Expr::value(u64_to_i64(now)?),
                        )
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    if result.rows_affected == 0 {
                        return Err(ApiError::not_found("CLI session not found"));
                    }
                    Ok(())
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                auth.cleanup_expired(now);
                let Some(session) = auth
                    .cli_sessions
                    .values_mut()
                    .find(|session| session.id == session_id && session.user_id == user_id)
                else {
                    return Err(ApiError::not_found("CLI session not found"));
                };
                session.revoked_at_unix = Some(now);
                Ok(())
            }
        }
    }
}

async fn create_cli_session_token_in_tx<C>(
    conn: &C,
    user_id: &str,
    now: u64,
) -> Result<CliSessionToken, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let session_id = random_prefixed_token(CLI_SESSION_ID_PREFIX)?;
    let session_token = random_prefixed_token(CLI_SESSION_TOKEN_PREFIX)?;
    let expires_at_unix = now + CLI_SESSION_TTL_SECS;
    entities::cli_session::Model {
        id: session_id,
        token_hash: token_hash(&session_token),
        user_id: user_id.to_string(),
        label: "Scope CLI".to_string(),
        created_at_unix: u64_to_i64(now)?,
        last_used_at_unix: None,
        expires_at_unix: u64_to_i64(expires_at_unix)?,
        revoked_at_unix: None,
    }
    .into_active_model()
    .insert(conn)
    .await
    .map_err(ApiError::internal)?;
    let user = load_user_by_id(conn, user_id).await?;
    Ok(CliSessionToken {
        session_token,
        expires_at_unix,
        identity: SessionIdentity::from(&user),
    })
}

#[cfg(test)]
fn create_cli_session_token_in_memory(
    memory: &super::MemoryMetadataStore,
    mut auth: std::sync::MutexGuard<'_, super::auth::MemoryAuthState>,
    user_id: String,
    now: u64,
) -> Result<CliSessionToken, ApiError> {
    let session_id = random_prefixed_token(CLI_SESSION_ID_PREFIX)?;
    let session_token = random_prefixed_token(CLI_SESSION_TOKEN_PREFIX)?;
    let expires_at_unix = now + CLI_SESSION_TTL_SECS;
    auth.cli_sessions.insert(
        token_hash(&session_token),
        MemoryCliSession {
            id: session_id,
            user_id: user_id.clone(),
            label: "Scope CLI".to_string(),
            created_at_unix: now,
            last_used_at_unix: None,
            expires_at_unix,
            revoked_at_unix: None,
        },
    );
    drop(auth);
    let catalog = memory
        .catalog
        .lock()
        .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?;
    let user = catalog
        .users
        .get(&user_id)
        .cloned()
        .ok_or_else(|| ApiError::internal_message("CLI session created for missing user"))?;
    Ok(CliSessionToken {
        session_token,
        expires_at_unix,
        identity: SessionIdentity::from(&user),
    })
}

fn browser_authorization_url(app_origin: &str, request_id: &str) -> Result<String, ApiError> {
    let mut url = Url::parse(app_origin)
        .map_err(|error| ApiError::internal_message(format!("invalid app origin: {error}")))?;
    url.set_path("/cli-login");
    url.set_query(None);
    url.query_pairs_mut().append_pair("request_id", request_id);
    Ok(url.to_string())
}

fn browser_callback_url(
    callback_url: &str,
    request_id: &str,
    callback_code: &str,
) -> Result<String, ApiError> {
    let mut url = validate_loopback_callback_url(callback_url)?;
    url.query_pairs_mut()
        .append_pair("request_id", request_id)
        .append_pair("code", callback_code);
    Ok(url.to_string())
}

fn validate_loopback_callback_url(callback_url: &str) -> Result<Url, ApiError> {
    let url = Url::parse(callback_url)
        .map_err(|_| ApiError::bad_request("CLI callback URL must be a valid URL"))?;
    if url.scheme() != "http" {
        return Err(ApiError::bad_request("CLI callback URL must use http"));
    }
    if url.port().is_none() {
        return Err(ApiError::bad_request(
            "CLI callback URL must include a port",
        ));
    }
    if url.path() != "/scope-cli-callback" {
        return Err(ApiError::bad_request(
            "CLI callback URL must use /scope-cli-callback",
        ));
    }
    if url.query().is_some() {
        return Err(ApiError::bad_request(
            "CLI callback URL must not include query parameters",
        ));
    }
    if url.fragment().is_some() {
        return Err(ApiError::bad_request(
            "CLI callback URL must not include a fragment",
        ));
    }
    let Some(host) = url.host_str() else {
        return Err(ApiError::bad_request(
            "CLI callback URL must include a host",
        ));
    };
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err(ApiError::bad_request(
            "CLI callback URL must use localhost or 127.0.0.1",
        ));
    }
    Ok(url)
}

async fn enforce_browser_login_start_limits<C>(conn: &C, now: u64) -> Result<(), ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let pending_count = entities::cli_browser_login::Entity::find()
        .count(conn)
        .await
        .map_err(ApiError::internal)?;
    if pending_count >= MAX_PENDING_BROWSER_LOGINS {
        return Err(ApiError::too_many_requests(
            "too many pending CLI browser logins",
        ));
    }

    let window_start = u64_to_i64(now.saturating_sub(BROWSER_LOGIN_START_WINDOW_SECS))?;
    let window_count = entities::cli_browser_login::Entity::find()
        .filter(entities::cli_browser_login::Column::CreatedAtUnix.gte(window_start))
        .count(conn)
        .await
        .map_err(ApiError::internal)?;
    if window_count >= MAX_BROWSER_LOGIN_STARTS_PER_WINDOW {
        return Err(ApiError::too_many_requests(
            "too many CLI browser login starts",
        ));
    }

    Ok(())
}

#[cfg(test)]
fn enforce_memory_browser_login_start_limits(
    auth: &super::auth::MemoryAuthState,
    now: u64,
) -> Result<(), ApiError> {
    if auth.cli_browser_logins.len() as u64 >= MAX_PENDING_BROWSER_LOGINS {
        return Err(ApiError::too_many_requests(
            "too many pending CLI browser logins",
        ));
    }

    let window_start = now.saturating_sub(BROWSER_LOGIN_START_WINDOW_SECS);
    let window_count = auth
        .cli_browser_logins
        .values()
        .filter(|login| login.created_at_unix >= window_start)
        .count() as u64;
    if window_count >= MAX_BROWSER_LOGIN_STARTS_PER_WINDOW {
        return Err(ApiError::too_many_requests(
            "too many CLI browser login starts",
        ));
    }

    Ok(())
}

fn cli_session_summary_from_model(
    session: entities::cli_session::Model,
) -> Result<CliSessionSummary, ApiError> {
    Ok(CliSessionSummary {
        id: session.id,
        label: session.label,
        created_at_unix: i64_to_u64(session.created_at_unix)?,
        last_used_at_unix: session.last_used_at_unix.map(i64_to_u64).transpose()?,
        expires_at_unix: i64_to_u64(session.expires_at_unix)?,
    })
}

#[cfg(test)]
fn cli_session_summary_from_memory(session: &MemoryCliSession) -> CliSessionSummary {
    CliSessionSummary {
        id: session.id.clone(),
        label: session.label.clone(),
        created_at_unix: session.created_at_unix,
        last_used_at_unix: session.last_used_at_unix,
        expires_at_unix: session.expires_at_unix,
    }
}
