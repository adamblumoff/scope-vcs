#[cfg(any(test, feature = "memory-metadata"))]
use super::auth::MemoryAuthState;
use super::{
    MetadataStore, MetadataStoreInner, acquire_metadata_write_lock, auth::load_user_by_id,
    entities, run_api_db_on,
};
#[cfg(any(test, feature = "memory-metadata"))]
use crate::domain::store::AppCatalog;
use crate::{
    auth::clerk::ClerkIdentity,
    domain::store::{AccountAccess, UserAccount},
    error::ApiError,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, TransactionTrait,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const CLERK_PROVIDER: &str = "clerk";

impl MetadataStore {
    pub(crate) fn resolve_existing_clerk_user(
        &self,
        identity: &ClerkIdentity,
    ) -> Result<Option<UserAccount>, ApiError> {
        let identity = identity.clone();
        match self.inner.as_ref() {
            MetadataStoreInner::Postgres { db, runtime } => {
                let db = Arc::clone(db);
                run_api_db_on(runtime, async move {
                    resolve_existing_clerk_user_in_tx(db.as_ref(), &identity).await
                })
            }
            #[cfg(any(test, feature = "memory-metadata"))]
            MetadataStoreInner::Memory(memory) => {
                let auth = memory
                    .auth
                    .lock()
                    .map_err(|_| ApiError::internal_message("auth lock is poisoned"))?;
                let catalog = memory
                    .catalog
                    .lock()
                    .map_err(|_| ApiError::internal_message("catalog lock is poisoned"))?;
                resolve_existing_clerk_user_in_memory(&auth, &catalog, &identity)
            }
        }
    }

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
            #[cfg(any(test, feature = "memory-metadata"))]
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
}

async fn resolve_existing_clerk_user_in_tx<C>(
    conn: &C,
    identity: &ClerkIdentity,
) -> Result<Option<UserAccount>, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let verified_email = verified_identity_email(identity)?;
    let Some(auth_identity) = entities::auth_identity::Entity::find()
        .filter(entities::auth_identity::Column::Provider.eq(CLERK_PROVIDER))
        .filter(entities::auth_identity::Column::Subject.eq(identity.user_id.clone()))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
    else {
        return Ok(None);
    };

    let mut user = load_user_by_id(conn, &auth_identity.user_id).await?;
    if let Some(email_owner) = load_user_by_email(conn, &verified_email).await?
        && email_owner.id != user.id
    {
        return Err(ApiError::conflict(
            "verified email belongs to another Scope user",
        ));
    }

    update_user_snapshot(&mut user, identity);
    Ok(Some(user))
}

async fn resolve_clerk_user_in_tx<C>(
    conn: &C,
    identity: &ClerkIdentity,
) -> Result<UserAccount, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let verified_email = verified_identity_email(identity)?;
    if let Some(auth_identity) = entities::auth_identity::Entity::find()
        .filter(entities::auth_identity::Column::Provider.eq(CLERK_PROVIDER))
        .filter(entities::auth_identity::Column::Subject.eq(identity.user_id.clone()))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
    {
        let mut user = load_user_by_id(conn, &auth_identity.user_id).await?;
        if let Some(email_owner) = load_user_by_email(conn, &verified_email).await?
            && email_owner.id != user.id
        {
            return Err(ApiError::conflict(
                "verified email belongs to another Scope user",
            ));
        }
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
        .find(|user| user.email.as_str() == verified_email)
        .or_else(|| users.iter().find(|user| user.id == user_id))
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

#[cfg(any(test, feature = "memory-metadata"))]
fn resolve_existing_clerk_user_in_memory(
    auth: &MemoryAuthState,
    catalog: &AppCatalog,
    identity: &ClerkIdentity,
) -> Result<Option<UserAccount>, ApiError> {
    let verified_email = verified_identity_email(identity)?;
    let key = auth_identity_key(CLERK_PROVIDER, &identity.user_id);
    let Some(mapped_user_id) = auth.auth_identities.get(&key) else {
        return Ok(None);
    };
    let mut user = catalog
        .users
        .get(mapped_user_id)
        .cloned()
        .ok_or_else(|| ApiError::internal_message("signed-in user was not persisted"))?;
    if let Some(email_owner) = catalog
        .users
        .values()
        .find(|candidate| candidate.email.as_str() == verified_email)
        && email_owner.id != user.id
    {
        return Err(ApiError::conflict(
            "verified email belongs to another Scope user",
        ));
    }

    update_user_snapshot(&mut user, identity);
    Ok(Some(user))
}

#[cfg(any(test, feature = "memory-metadata"))]
fn resolve_clerk_user_in_memory(
    auth: &mut MemoryAuthState,
    catalog: &mut AppCatalog,
    identity: &ClerkIdentity,
) -> Result<UserAccount, ApiError> {
    let verified_email = verified_identity_email(identity)?;
    let key = auth_identity_key(CLERK_PROVIDER, &identity.user_id);
    let mapped_user_id = auth.auth_identities.get(&key).cloned();
    let email_user_id = catalog
        .users
        .values()
        .find(|user| user.email.as_str() == verified_email)
        .map(|user| user.id.clone());
    if let (Some(mapped_user_id), Some(email_user_id)) = (&mapped_user_id, &email_user_id)
        && mapped_user_id != email_user_id
    {
        return Err(ApiError::conflict(
            "verified email belongs to another Scope user",
        ));
    }
    let user_id = mapped_user_id
        .or(email_user_id)
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

async fn load_user_by_email<C>(conn: &C, email: &str) -> Result<Option<UserAccount>, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    entities::user::Entity::find()
        .filter(entities::user::Column::Email.eq(email.to_string()))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(|user| user.try_into_domain())
        .transpose()
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

fn verified_identity_email(identity: &ClerkIdentity) -> Result<String, ApiError> {
    if !identity.email_verified {
        return Err(ApiError::unauthorized("verified email required"));
    }
    let email = identity
        .email
        .as_deref()
        .map(normalize_email)
        .unwrap_or_default();
    if email.is_empty() {
        return Err(ApiError::unauthorized("verified email required"));
    }
    Ok(email)
}

pub(crate) fn scope_user_id_for_auth_identity(provider: &str, subject: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider.as_bytes());
    hasher.update(b"\0");
    hasher.update(subject.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("scope_usr_{}", &digest[..24])
}

#[cfg(any(test, feature = "memory-metadata"))]
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
