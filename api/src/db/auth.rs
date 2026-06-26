use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, entities, run_api_db_on,
};
#[cfg(test)]
use crate::domain::store::AppCatalog;
use crate::{
    auth::{
        clerk::ClerkIdentity,
        device::{
            DEVICE_LOGIN_START_WINDOW_SECS, DeviceLoginPoll, DeviceLoginStart,
            MAX_DEVICE_LOGIN_STARTS_PER_WINDOW, MAX_PENDING_DEVICE_LOGINS, random_prefixed_token,
            random_user_code,
        },
        tokens::token_hash,
    },
    config::{
        CLI_DEVICE_CODE_PREFIX, CLI_DEVICE_LOGIN_POLL_INTERVAL_SECS, CLI_DEVICE_LOGIN_TTL_SECS,
        CLI_SESSION_ID_PREFIX, CLI_SESSION_TOKEN_PREFIX, CLI_SESSION_TTL_SECS,
    },
    domain::store::{AccountAccess, UserAccount},
    error::ApiError,
    http::responses::SessionIdentity,
    persistence::unix_now,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    TransactionTrait, sea_query::Expr,
};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::collections::BTreeMap;
use std::sync::Arc;

const CLERK_PROVIDER: &str = "clerk";

#[cfg(test)]
#[derive(Default)]
pub(super) struct MemoryAuthState {
    auth_identities: BTreeMap<String, String>,
    cli_device_logins: BTreeMap<String, MemoryDeviceLogin>,
    cli_device_code_by_user_code_hash: BTreeMap<String, String>,
    pub(super) cli_browser_logins: BTreeMap<String, MemoryBrowserLogin>,
    pub(super) cli_exchange_grants: BTreeMap<String, MemoryExchangeGrant>,
    pub(super) cli_sessions: BTreeMap<String, MemoryCliSession>,
}

#[cfg(test)]
struct MemoryDeviceLogin {
    user_code_hash: String,
    created_at_unix: u64,
    expires_at_unix: u64,
    completed_user_id: Option<String>,
    completed_at_unix: Option<u64>,
    consumed_at_unix: Option<u64>,
}

#[cfg(test)]
pub(super) struct MemoryBrowserLogin {
    pub(super) request_secret_hash: String,
    pub(super) callback_url: String,
    pub(super) callback_code_hash: Option<String>,
    pub(super) created_at_unix: u64,
    pub(super) expires_at_unix: u64,
    pub(super) completed_user_id: Option<String>,
    pub(super) completed_at_unix: Option<u64>,
    pub(super) consumed_at_unix: Option<u64>,
}

#[cfg(test)]
pub(super) struct MemoryExchangeGrant {
    pub(super) user_id: String,
    pub(super) expires_at_unix: u64,
    pub(super) consumed_at_unix: Option<u64>,
}

#[cfg(test)]
pub(super) struct MemoryCliSession {
    pub(super) id: String,
    pub(super) user_id: String,
    pub(super) label: String,
    pub(super) created_at_unix: u64,
    pub(super) last_used_at_unix: Option<u64>,
    pub(super) expires_at_unix: u64,
    pub(super) revoked_at_unix: Option<u64>,
}

impl MetadataStore {
    pub(crate) fn resolve_clerk_user(
        &self,
        identity: &ClerkIdentity,
    ) -> Result<UserAccount, ApiError> {
        let identity = identity.clone();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    let user = resolve_clerk_user_in_tx(&tx, &identity).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(user)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                let mut catalog = memory
                    .catalog
                    .lock()
                    .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?;
                resolve_clerk_user_in_memory(&mut auth, &mut catalog, &identity)
            }
        }
    }

    pub(crate) fn start_cli_device_login(
        &self,
        app_origin: &str,
    ) -> Result<DeviceLoginStart, ApiError> {
        let app_origin = app_origin.trim_end_matches('/').to_string();
        let now = unix_now()?;
        let expires_at_unix = now + CLI_DEVICE_LOGIN_TTL_SECS;

        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;
                    cleanup_expired_cli_rows(&tx, now).await?;
                    enforce_device_login_start_limits(&tx, now).await?;

                    let (device_code, user_code) = loop {
                        let device_code = random_prefixed_token(CLI_DEVICE_CODE_PREFIX)?;
                        let user_code = random_user_code()?;
                        let device_code_hash = token_hash(&device_code);
                        let user_code_hash = token_hash(&normalize_user_code(&user_code));
                        let device_exists =
                            entities::cli_device_login::Entity::find_by_id(device_code_hash)
                                .one(&tx)
                                .await
                                .map_err(ApiError::internal)?
                                .is_some();
                        let user_exists = entities::cli_device_login::Entity::find()
                            .filter(
                                entities::cli_device_login::Column::UserCodeHash.eq(user_code_hash),
                            )
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
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                auth.cleanup_expired(now);
                auth.enforce_start_limits(now)?;

                let (device_code, user_code) = loop {
                    let device_code = random_prefixed_token(CLI_DEVICE_CODE_PREFIX)?;
                    let user_code = random_user_code()?;
                    let device_code_hash = token_hash(&device_code);
                    let user_code_hash = token_hash(&normalize_user_code(&user_code));
                    if !auth.cli_device_logins.contains_key(&device_code_hash)
                        && !auth
                            .cli_device_code_by_user_code_hash
                            .contains_key(&user_code_hash)
                    {
                        break (device_code, user_code);
                    }
                };
                let device_code_hash = token_hash(&device_code);
                let user_code_hash = token_hash(&normalize_user_code(&user_code));
                auth.cli_device_code_by_user_code_hash
                    .insert(user_code_hash.clone(), device_code_hash.clone());
                auth.cli_device_logins.insert(
                    device_code_hash,
                    MemoryDeviceLogin {
                        user_code_hash,
                        created_at_unix: now,
                        expires_at_unix,
                        completed_user_id: None,
                        completed_at_unix: None,
                        consumed_at_unix: None,
                    },
                );

                Ok(DeviceLoginStart {
                    device_code,
                    user_code,
                    verification_url: format!("{app_origin}/cli-login"),
                    expires_at_unix,
                    poll_interval_secs: CLI_DEVICE_LOGIN_POLL_INTERVAL_SECS,
                })
            }
        }
    }

    pub(crate) fn complete_cli_device_login(
        &self,
        raw_user_code: &str,
        user: &UserAccount,
    ) -> Result<(), ApiError> {
        let user_code_hash = token_hash(&normalize_user_code(raw_user_code));
        let user_id = user.id.clone();
        let now = unix_now()?;
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
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

                    if now >= i64_to_u64(login.expires_at_unix)? {
                        entities::cli_device_login::Entity::delete_by_id(login.device_code_hash)
                            .exec(&tx)
                            .await
                            .map_err(ApiError::internal)?;
                        return Err(ApiError::conflict("CLI login code expired"));
                    }
                    if login.completed_user_id.is_some() {
                        return Err(ApiError::conflict("CLI login already completed"));
                    }

                    cleanup_expired_cli_rows(&tx, now).await?;
                    entities::cli_device_login::Entity::update_many()
                        .filter(
                            entities::cli_device_login::Column::DeviceCodeHash
                                .eq(login.device_code_hash),
                        )
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
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                let Some(device_code_hash) = auth
                    .cli_device_code_by_user_code_hash
                    .get(&user_code_hash)
                    .cloned()
                else {
                    return Err(ApiError::not_found("CLI login code not found"));
                };
                let Some(login) = auth.cli_device_logins.get_mut(&device_code_hash) else {
                    return Err(ApiError::not_found("CLI login code not found"));
                };
                if now >= login.expires_at_unix {
                    auth.remove_device_login(&device_code_hash);
                    return Err(ApiError::conflict("CLI login code expired"));
                }
                if login.completed_user_id.is_some() {
                    return Err(ApiError::conflict("CLI login already completed"));
                }

                login.completed_user_id = Some(user_id);
                login.completed_at_unix = Some(now);
                Ok(())
            }
        }
    }

    pub(crate) fn poll_cli_device_login(
        &self,
        device_code: &str,
    ) -> Result<DeviceLoginPoll, ApiError> {
        let device_code_hash = token_hash(device_code);
        let now = unix_now()?;
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
                    acquire_metadata_write_lock(&tx).await?;

                    let Some(login) =
                        entities::cli_device_login::Entity::find_by_id(device_code_hash)
                            .one(&tx)
                            .await
                            .map_err(ApiError::internal)?
                    else {
                        return Err(ApiError::not_found("CLI device login not found"));
                    };
                    if now >= i64_to_u64(login.expires_at_unix)? {
                        entities::cli_device_login::Entity::delete_by_id(login.device_code_hash)
                            .exec(&tx)
                            .await
                            .map_err(ApiError::internal)?;
                        return Err(ApiError::conflict("CLI device login expired"));
                    }
                    if login.consumed_at_unix.is_some() {
                        return Err(ApiError::conflict("CLI device login already consumed"));
                    }
                    let Some(user_id) = login.completed_user_id else {
                        let expires_at_unix = i64_to_u64(login.expires_at_unix)?;
                        tx.commit().await.map_err(ApiError::internal)?;
                        return Ok(DeviceLoginPoll::Pending { expires_at_unix });
                    };

                    cleanup_expired_cli_rows(&tx, now).await?;
                    let session_id = random_prefixed_token(CLI_SESSION_ID_PREFIX)?;
                    let session_token = random_prefixed_token(CLI_SESSION_TOKEN_PREFIX)?;
                    let session_expires_at_unix = now + CLI_SESSION_TTL_SECS;
                    entities::cli_session::Model {
                        id: session_id,
                        token_hash: token_hash(&session_token),
                        user_id: user_id.clone(),
                        label: "Scope CLI".to_string(),
                        created_at_unix: u64_to_i64(now)?,
                        last_used_at_unix: None,
                        expires_at_unix: u64_to_i64(session_expires_at_unix)?,
                        revoked_at_unix: None,
                    }
                    .into_active_model()
                    .insert(&tx)
                    .await
                    .map_err(ApiError::internal)?;
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
                    let user = load_user_by_id(&tx, &user_id).await?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(DeviceLoginPoll::Complete {
                        session_token,
                        expires_at_unix: session_expires_at_unix,
                        identity: SessionIdentity::from(&user),
                    })
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                let Some(login) = auth.cli_device_logins.get_mut(&device_code_hash) else {
                    return Err(ApiError::not_found("CLI device login not found"));
                };
                if now >= login.expires_at_unix {
                    auth.remove_device_login(&device_code_hash);
                    return Err(ApiError::conflict("CLI device login expired"));
                }
                if login.consumed_at_unix.is_some() {
                    return Err(ApiError::conflict("CLI device login already consumed"));
                }
                let Some(user_id) = login.completed_user_id.clone() else {
                    return Ok(DeviceLoginPoll::Pending {
                        expires_at_unix: login.expires_at_unix,
                    });
                };

                login.consumed_at_unix = Some(now);
                let session_id = random_prefixed_token(CLI_SESSION_ID_PREFIX)?;
                let session_token = random_prefixed_token(CLI_SESSION_TOKEN_PREFIX)?;
                let session_expires_at_unix = now + CLI_SESSION_TTL_SECS;
                auth.cli_sessions.insert(
                    token_hash(&session_token),
                    MemoryCliSession {
                        id: session_id,
                        user_id: user_id.clone(),
                        label: "Scope CLI".to_string(),
                        created_at_unix: now,
                        last_used_at_unix: None,
                        expires_at_unix: session_expires_at_unix,
                        revoked_at_unix: None,
                    },
                );
                drop(auth);
                let catalog = memory
                    .catalog
                    .lock()
                    .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?;
                let user = catalog.users.get(&user_id).cloned().ok_or_else(|| {
                    ApiError::internal_message("CLI login completed for missing user")
                })?;
                Ok(DeviceLoginPoll::Complete {
                    session_token,
                    expires_at_unix: session_expires_at_unix,
                    identity: SessionIdentity::from(&user),
                })
            }
        }
    }

    pub(crate) fn verify_cli_session_token(
        &self,
        session_token: &str,
    ) -> Result<UserAccount, ApiError> {
        let token_hash = token_hash(session_token);
        let now = unix_now()?;
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
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
                    if now >= i64_to_u64(session.expires_at_unix)? {
                        entities::cli_session::Entity::delete_by_id(session.id)
                            .exec(&tx)
                            .await
                            .map_err(ApiError::internal)?;
                        return Err(ApiError::unauthorized("CLI token expired"));
                    }
                    if session.revoked_at_unix.is_some() {
                        return Err(ApiError::unauthorized("CLI session revoked"));
                    }
                    cleanup_expired_cli_rows(&tx, now).await?;
                    let user = load_user_by_id(&tx, &session.user_id).await?;
                    entities::cli_session::Entity::update_many()
                        .filter(entities::cli_session::Column::Id.eq(session.id))
                        .col_expr(
                            entities::cli_session::Column::LastUsedAtUnix,
                            Expr::value(u64_to_i64(now)?),
                        )
                        .exec(&tx)
                        .await
                        .map_err(ApiError::internal)?;
                    tx.commit().await.map_err(ApiError::internal)?;
                    Ok(user)
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                let Some(session) = auth.cli_sessions.get_mut(&token_hash) else {
                    return Err(ApiError::unauthorized("invalid CLI token"));
                };
                if now >= session.expires_at_unix {
                    auth.cli_sessions.remove(&token_hash);
                    return Err(ApiError::unauthorized("CLI token expired"));
                }
                if session.revoked_at_unix.is_some() {
                    return Err(ApiError::unauthorized("CLI session revoked"));
                }
                let user_id = session.user_id.clone();
                session.last_used_at_unix = Some(now);
                auth.cleanup_expired(now);
                drop(auth);
                let catalog = memory
                    .catalog
                    .lock()
                    .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?;
                catalog
                    .users
                    .get(&user_id)
                    .cloned()
                    .ok_or_else(|| ApiError::unauthorized("invalid CLI token"))
            }
        }
    }

    pub(crate) fn revoke_cli_session_token(&self, session_token: &str) -> Result<(), ApiError> {
        let token_hash = token_hash(session_token);
        let now = unix_now()?;
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
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
                    if now >= i64_to_u64(session.expires_at_unix)? {
                        entities::cli_session::Entity::delete_by_id(session.id)
                            .exec(&tx)
                            .await
                            .map_err(ApiError::internal)?;
                        return Err(ApiError::unauthorized("CLI token expired"));
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
                })
            }
            #[cfg(test)]
            MetadataStoreInner::Memory(memory) => {
                let mut auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                let Some(session) = auth.cli_sessions.get_mut(&token_hash) else {
                    return Err(ApiError::unauthorized("invalid CLI token"));
                };
                if now >= session.expires_at_unix {
                    auth.cli_sessions.remove(&token_hash);
                    return Err(ApiError::unauthorized("CLI token expired"));
                }
                session.revoked_at_unix = Some(now);
                Ok(())
            }
        }
    }
}

async fn resolve_clerk_user_in_tx<C>(
    conn: &C,
    identity: &ClerkIdentity,
) -> Result<UserAccount, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    if let Some(auth_identity) = entities::auth_identity::Entity::find()
        .filter(entities::auth_identity::Column::Provider.eq(CLERK_PROVIDER))
        .filter(entities::auth_identity::Column::Subject.eq(identity.user_id.clone()))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
    {
        let mut user = load_user_by_id(conn, &auth_identity.user_id).await?;
        update_user_snapshot(&mut user, identity);
        entities::user::Model::from_domain(&user)
            .into_active_model()
            .update(conn)
            .await
            .map_err(ApiError::internal)?;
        return Ok(user);
    }

    let users = entities::user::Entity::find()
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|user| user.try_into_domain())
        .collect::<Result<Vec<_>, _>>()?;
    let user_id = scope_user_id_for_auth_identity(CLERK_PROVIDER, &identity.user_id);
    let mut user = users
        .iter()
        .find(|user| user.id == user_id)
        .cloned()
        .unwrap_or_else(|| {
            let preferred = preferred_user_handle(identity);
            UserAccount {
                id: user_id.clone(),
                handle: unique_user_handle(users.iter(), &preferred, &user_id),
                email: String::new(),
                email_verified: false,
                access: AccountAccess::Member,
            }
        });
    update_user_snapshot(&mut user, identity);

    if users.iter().any(|existing| existing.id == user.id) {
        entities::user::Model::from_domain(&user)
            .into_active_model()
            .update(conn)
            .await
            .map_err(ApiError::internal)?;
    } else {
        entities::user::Model::from_domain(&user)
            .into_active_model()
            .insert(conn)
            .await
            .map_err(ApiError::internal)?;
    }
    entities::auth_identity::Model {
        provider: CLERK_PROVIDER.to_string(),
        subject: identity.user_id.clone(),
        user_id: user.id.clone(),
    }
    .into_active_model()
    .insert(conn)
    .await
    .map_err(ApiError::internal)?;

    Ok(user)
}

#[cfg(test)]
fn resolve_clerk_user_in_memory(
    auth: &mut MemoryAuthState,
    catalog: &mut AppCatalog,
    identity: &ClerkIdentity,
) -> Result<UserAccount, ApiError> {
    let key = auth_identity_key(CLERK_PROVIDER, &identity.user_id);
    let user_id = auth
        .auth_identities
        .get(&key)
        .cloned()
        .unwrap_or_else(|| scope_user_id_for_auth_identity(CLERK_PROVIDER, &identity.user_id));
    let mut user = catalog.users.get(&user_id).cloned().unwrap_or_else(|| {
        let preferred = preferred_user_handle(identity);
        UserAccount {
            id: user_id.clone(),
            handle: unique_user_handle(catalog.users.values(), &preferred, &user_id),
            email: String::new(),
            email_verified: false,
            access: AccountAccess::Member,
        }
    });
    update_user_snapshot(&mut user, identity);
    catalog.users.insert(user.id.clone(), user.clone());
    auth.auth_identities.insert(key, user.id.clone());
    Ok(user)
}

pub(super) async fn cleanup_expired_cli_rows<C>(conn: &C, now: u64) -> Result<(), ApiError>
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
    if pending_count >= MAX_PENDING_DEVICE_LOGINS {
        return Err(ApiError::too_many_requests(
            "too many pending CLI device logins",
        ));
    }

    let window_start = u64_to_i64(now.saturating_sub(DEVICE_LOGIN_START_WINDOW_SECS))?;
    let window_count = entities::cli_device_login::Entity::find()
        .filter(entities::cli_device_login::Column::CreatedAtUnix.gte(window_start))
        .count(conn)
        .await
        .map_err(ApiError::internal)?;
    if window_count >= MAX_DEVICE_LOGIN_STARTS_PER_WINDOW {
        return Err(ApiError::too_many_requests(
            "too many CLI device login starts",
        ));
    }

    Ok(())
}

#[cfg(test)]
impl MemoryAuthState {
    fn enforce_start_limits(&self, now: u64) -> Result<(), ApiError> {
        if self.cli_device_logins.len() as u64 >= MAX_PENDING_DEVICE_LOGINS {
            return Err(ApiError::too_many_requests(
                "too many pending CLI device logins",
            ));
        }

        let window_start = now.saturating_sub(DEVICE_LOGIN_START_WINDOW_SECS);
        let window_count = self
            .cli_device_logins
            .values()
            .filter(|login| login.created_at_unix >= window_start)
            .count() as u64;
        if window_count >= MAX_DEVICE_LOGIN_STARTS_PER_WINDOW {
            return Err(ApiError::too_many_requests(
                "too many CLI device login starts",
            ));
        }

        Ok(())
    }

    pub(super) fn cleanup_expired(&mut self, now: u64) {
        let expired_device_codes = self
            .cli_device_logins
            .iter()
            .filter(|(_, login)| now >= login.expires_at_unix)
            .map(|(device_code_hash, _)| device_code_hash.clone())
            .collect::<Vec<_>>();
        for device_code_hash in expired_device_codes {
            self.remove_device_login(&device_code_hash);
        }
        self.cli_browser_logins
            .retain(|_, login| now < login.expires_at_unix);
        self.cli_exchange_grants
            .retain(|_, grant| now < grant.expires_at_unix);
        self.cli_sessions
            .retain(|_, session| now < session.expires_at_unix);
    }

    fn remove_device_login(&mut self, device_code_hash: &str) {
        if let Some(login) = self.cli_device_logins.remove(device_code_hash) {
            self.cli_device_code_by_user_code_hash
                .remove(&login.user_code_hash);
        }
    }
}

pub(super) async fn load_user_by_id<C>(conn: &C, user_id: &str) -> Result<UserAccount, ApiError>
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

fn update_user_snapshot(user: &mut UserAccount, identity: &ClerkIdentity) {
    user.email = identity
        .email
        .as_deref()
        .map(normalize_email)
        .unwrap_or_default();
    user.email_verified = identity.email_verified;
    user.access = AccountAccess::Member;
}

pub(crate) fn scope_user_id_for_auth_identity(provider: &str, subject: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider.as_bytes());
    hasher.update(b"\0");
    hasher.update(subject.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("scope_usr_{}", &digest[..24])
}

#[cfg(test)]
fn auth_identity_key(provider: &str, subject: &str) -> String {
    format!("{provider}\0{subject}")
}

fn preferred_user_handle(identity: &ClerkIdentity) -> String {
    let fallback = handle_suffix(&identity.user_id);
    let raw = identity
        .email
        .as_deref()
        .filter(|_| identity.email_verified)
        .and_then(|email| email.split('@').next())
        .filter(|local| !local.trim().is_empty())
        .unwrap_or(&fallback);

    normalize_handle(raw).unwrap_or(fallback)
}

fn handle_suffix(user_id: &str) -> String {
    let suffix = user_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(8)
        .collect::<String>();
    if suffix.is_empty() {
        "user".to_string()
    } else {
        format!("user-{suffix}")
    }
}

fn unique_user_handle<'a>(
    users: impl IntoIterator<Item = &'a UserAccount>,
    preferred: &str,
    user_id: &str,
) -> String {
    let users = users.into_iter().collect::<Vec<_>>();
    let base = normalize_handle(preferred).unwrap_or_else(|| "user".to_string());
    if handle_is_available(&users, &base, user_id) {
        return base;
    }

    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if handle_is_available(&users, &candidate, user_id) {
            return candidate;
        }
    }

    unreachable!("infinite suffix search must find an available handle")
}

fn handle_is_available(users: &[&UserAccount], handle: &str, user_id: &str) -> bool {
    users
        .iter()
        .all(|user| user.id == user_id || user.handle != handle)
}

fn normalize_handle(value: &str) -> Option<String> {
    let mut handle = String::new();
    let mut last_was_separator = false;
    for byte in value.trim().bytes() {
        let next = if byte.is_ascii_alphanumeric() {
            last_was_separator = false;
            Some(byte.to_ascii_lowercase() as char)
        } else if matches!(byte, b'-' | b'_') && !last_was_separator {
            last_was_separator = true;
            Some('-')
        } else {
            None
        };

        if let Some(next) = next {
            handle.push(next);
        }
    }

    let handle = handle.trim_matches('-').to_string();
    if handle.is_empty() || handle.len() > 40 {
        None
    } else {
        Some(handle)
    }
}

fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn normalize_user_code(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-')
        .flat_map(char::to_uppercase)
        .collect()
}

pub(super) fn u64_to_i64(value: u64) -> Result<i64, ApiError> {
    i64::try_from(value).map_err(ApiError::internal)
}

pub(super) fn i64_to_u64(value: i64) -> Result<u64, ApiError> {
    u64::try_from(value).map_err(ApiError::internal)
}
