use super::{MetadataStore, acquire_aggregate_lock, auth::load_user_by_id, entities};
use crate::{auth::clerk::ClerkIdentity, domain::store::UserAccount, error::ApiError};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
    TransactionTrait,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const CLERK_PROVIDER: &str = "clerk";

impl MetadataStore {
    pub async fn resolve_existing_clerk_user(
        &self,
        identity: &ClerkIdentity,
    ) -> Result<Option<UserAccount>, ApiError> {
        let identity = identity.clone();
        let db = Arc::clone(&self.db);
        resolve_existing_clerk_user_in_tx(db.as_ref(), &identity).await
    }

    pub async fn resolve_clerk_user(
        &self,
        identity: &ClerkIdentity,
    ) -> Result<UserAccount, ApiError> {
        let identity = identity.clone();
        let verified_email = verified_identity_email(&identity)?;
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        let identity_key = format!("{CLERK_PROVIDER}:{}", identity.user_id);
        acquire_aggregate_lock(&tx, "auth-identity", &identity_key).await?;
        acquire_aggregate_lock(&tx, "auth-email", &verified_email).await?;
        let user = resolve_clerk_user_in_tx(&tx, &identity, &verified_email).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(user)
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
    verified_email: &str,
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
        if let Some(email_owner) = load_user_by_email(conn, verified_email).await?
            && email_owner.id != user.id
        {
            return Err(ApiError::conflict(
                "verified email belongs to another Scope user",
            ));
        }
        update_user_snapshot(&mut user, identity);
        update_user(conn, &user).await?;
        return Ok(user);
    }

    let user_id = scope_user_id_for_auth_identity(CLERK_PROVIDER, &identity.user_id);
    let existing_user = match load_user_by_email(conn, verified_email).await? {
        Some(user) => Some(user),
        None => entities::user::Entity::find_by_id(user_id.clone())
            .one(conn)
            .await
            .map_err(ApiError::internal)?
            .map(entities::user::Model::try_into_domain)
            .transpose()?,
    };
    let is_existing = existing_user.is_some();
    let mut user = match existing_user {
        Some(user) => user,
        None => {
            let preferred = preferred_user_handle(identity);
            acquire_aggregate_lock(conn, "auth-handle-allocation", "global").await?;
            UserAccount {
                id: user_id.clone(),
                handle: unique_user_handle(conn, &preferred, &user_id).await?,
                email: String::new(),
                email_verified: false,
            }
        }
    };
    update_user_snapshot(&mut user, identity);

    if is_existing {
        update_user(conn, &user).await?;
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

async fn update_user<C>(conn: &C, user: &UserAccount) -> Result<(), ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let mut active = entities::user::Model::from_domain(user).into_active_model();
    active.handle = Set(user.handle.clone());
    active.email = Set(user.email.clone());
    active.email_verified = Set(user.email_verified);
    active.update(conn).await.map_err(ApiError::internal)?;
    Ok(())
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

pub fn scope_user_id_for_auth_identity(provider: &str, subject: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider.as_bytes());
    hasher.update(b"\0");
    hasher.update(subject.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("scope_usr_{}", &digest[..24])
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

async fn unique_user_handle<C>(conn: &C, preferred: &str, user_id: &str) -> Result<String, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let base = normalize_handle(preferred).unwrap_or_else(|| "user".to_string());
    if handle_is_available(conn, &base, user_id).await? {
        return Ok(base);
    }

    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if handle_is_available(conn, &candidate, user_id).await? {
            return Ok(candidate);
        }
    }

    unreachable!("infinite suffix search must find an available handle")
}

async fn handle_is_available<C>(conn: &C, handle: &str, user_id: &str) -> Result<bool, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let owner = entities::user::Entity::find()
        .filter(entities::user::Column::Handle.eq(handle.to_string()))
        .one(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(owner.is_none_or(|user| user.id == user_id))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::TestDatabaseTarget;
    use std::collections::HashSet;
    use tokio::{sync::Barrier, task::JoinSet};

    async fn resolve_concurrently(
        store: MetadataStore,
        identities: Vec<ClerkIdentity>,
    ) -> Vec<UserAccount> {
        let barrier = Arc::new(Barrier::new(identities.len()));
        let mut tasks = JoinSet::new();
        for identity in identities {
            let store = store.clone();
            let barrier = Arc::clone(&barrier);
            tasks.spawn(async move {
                barrier.wait().await;
                store.resolve_clerk_user(&identity).await
            });
        }

        let mut users = Vec::new();
        while let Some(result) = tasks.join_next().await {
            users.push(result.unwrap().unwrap());
        }
        users
    }

    #[tokio::test]
    async fn concurrent_subjects_merge_the_same_verified_email() {
        let target = TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let identities = (0..6)
            .map(|index| ClerkIdentity {
                user_id: format!("subject-{index}"),
                email: Some("Shared@Example.com".to_string()),
                email_verified: true,
            })
            .collect();

        let users = resolve_concurrently(store.clone(), identities).await;

        assert!(users.iter().all(|user| user.id == users[0].id));
        assert_eq!(store.user_count_for_tests().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn concurrent_subjects_allocate_unique_preferred_handles() {
        let target = TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let identities = (0..6)
            .map(|index| ClerkIdentity {
                user_id: format!("subject-{index}"),
                email: Some(format!("shared@{index}.example.com")),
                email_verified: true,
            })
            .collect();

        let users = resolve_concurrently(store, identities).await;
        let user_ids = users.iter().map(|user| &user.id).collect::<HashSet<_>>();
        let handles = users
            .iter()
            .map(|user| &user.handle)
            .collect::<HashSet<_>>();

        assert_eq!(user_ids.len(), users.len());
        assert_eq!(handles.len(), users.len());
        assert!(handles.contains(&"shared".to_string()));
        assert!(handles.iter().all(|handle| handle.starts_with("shared")));
    }
}
