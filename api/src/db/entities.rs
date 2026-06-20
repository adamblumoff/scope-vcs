use crate::db::{decode_json, encode_json};
use crate::domain::policy::{Policy, Visibility};
use crate::domain::projection::SourceGraph;
use crate::domain::store::{
    AccountAccess, FirstPushToken, GitPushToken, InvitationState, PendingImport, RepoInvitation,
    RepoMembership, RepoPublicationState, RepoRecord, RepoRole, RepoSettings, StagedRepoUpdate,
    StoredRepository, UserAccount,
};
use crate::error::ApiError;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

fn encode_enum<T: serde::Serialize>(value: T) -> Result<String, ApiError> {
    match serde_json::to_value(value).map_err(ApiError::internal)? {
        serde_json::Value::String(value) => Ok(value),
        _ => Err(ApiError::internal_message(
            "enum did not serialize to string",
        )),
    }
}

fn decode_enum<T: serde::de::DeserializeOwned>(value: String) -> Result<T, ApiError> {
    serde_json::from_value(serde_json::Value::String(value)).map_err(ApiError::internal)
}

#[derive(Serialize, Deserialize)]
struct PersistedFirstPushToken {
    token_hash: String,
    owner_user_id: String,
    created_at_unix: u64,
    expires_at_unix: u64,
    used_at_unix: Option<u64>,
}

impl From<&FirstPushToken> for PersistedFirstPushToken {
    fn from(token: &FirstPushToken) -> Self {
        Self {
            token_hash: token.token_hash.clone(),
            owner_user_id: token.owner_user_id.clone(),
            created_at_unix: token.created_at_unix,
            expires_at_unix: token.expires_at_unix,
            used_at_unix: token.used_at_unix,
        }
    }
}

impl From<PersistedFirstPushToken> for FirstPushToken {
    fn from(token: PersistedFirstPushToken) -> Self {
        Self {
            token_hash: token.token_hash,
            secret: None,
            owner_user_id: token.owner_user_id,
            created_at_unix: token.created_at_unix,
            expires_at_unix: token.expires_at_unix,
            used_at_unix: token.used_at_unix,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_first_push_token_never_stores_plaintext_secret() {
        let token = FirstPushToken {
            token_hash: "hash".to_string(),
            secret: Some("scope-setup-secret".to_string()),
            owner_user_id: "user-1".to_string(),
            created_at_unix: 10,
            expires_at_unix: 20,
            used_at_unix: None,
        };

        let persisted = PersistedFirstPushToken::from(&token);
        let json = serde_json::to_value(&persisted).expect("token serializes");
        assert!(json.get("secret").is_none());

        let rehydrated = FirstPushToken::from(
            serde_json::from_value::<PersistedFirstPushToken>(json).expect("token deserializes"),
        );
        assert_eq!(rehydrated.secret, None);
    }
}

pub(crate) mod user {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_users")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        #[sea_orm(unique)]
        pub handle: String,
        pub email: String,
        pub email_verified: bool,
        pub access: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(user: &UserAccount) -> Self {
            Self {
                id: user.id.clone(),
                handle: user.handle.clone(),
                email: user.email.clone(),
                email_verified: user.email_verified,
                access: encode_enum(user.access).expect("AccountAccess serializes to a string"),
            }
        }

        pub(crate) fn try_into_domain(self) -> Result<UserAccount, ApiError> {
            Ok(UserAccount {
                id: self.id,
                handle: self.handle,
                email: self.email,
                email_verified: self.email_verified,
                access: decode_enum::<AccountAccess>(self.access)?,
            })
        }
    }
}

pub(crate) mod repository {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repositories")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub owner_handle: String,
        pub name: String,
        pub owner_user_id: String,
        pub publication_state: String,
        pub default_visibility: String,
        pub settings: Json,
        pub first_push_token: Option<Json>,
        pub git_push_token: Option<Json>,
        pub pending_import: Option<Json>,
        pub policy: Json,
        pub graph: Json,
        pub staged_update: Option<Json>,
        pub invitations: Json,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(repo: &StoredRepository) -> Result<Self, ApiError> {
            Ok(Self {
                id: repo.record.id.clone(),
                owner_handle: repo.record.owner_handle.clone(),
                name: repo.record.name.clone(),
                owner_user_id: repo.record.owner_user_id.clone(),
                publication_state: encode_enum(repo.record.publication_state)?,
                default_visibility: encode_enum(repo.record.default_visibility)?,
                settings: encode_json(&repo.settings)?,
                first_push_token: repo
                    .first_push_token
                    .as_ref()
                    .map(PersistedFirstPushToken::from)
                    .map(|token| encode_json(&token))
                    .transpose()?,
                git_push_token: repo.git_push_token.as_ref().map(encode_json).transpose()?,
                pending_import: repo.pending_import.as_ref().map(encode_json).transpose()?,
                policy: encode_json(&repo.policy)?,
                graph: encode_json(&repo.graph)?,
                staged_update: repo.staged_update.as_ref().map(encode_json).transpose()?,
                invitations: encode_json(&repo.invitations)?,
            })
        }

        pub(crate) fn try_into_domain(
            self,
            memberships: Vec<RepoMembership>,
        ) -> Result<StoredRepository, ApiError> {
            let publication_state = decode_enum::<RepoPublicationState>(self.publication_state)?;
            let default_visibility = decode_enum::<Visibility>(self.default_visibility)?;
            Ok(StoredRepository {
                record: RepoRecord {
                    id: self.id.clone(),
                    owner_handle: self.owner_handle,
                    name: self.name,
                    owner_user_id: self.owner_user_id,
                    publication_state,
                    default_visibility,
                },
                settings: decode_json::<RepoSettings>(self.settings)?,
                first_push_token: self
                    .first_push_token
                    .map(decode_json::<PersistedFirstPushToken>)
                    .transpose()?
                    .map(FirstPushToken::from),
                git_push_token: self
                    .git_push_token
                    .map(decode_json::<GitPushToken>)
                    .transpose()?,
                pending_import: self
                    .pending_import
                    .map(decode_json::<PendingImport>)
                    .transpose()?,
                policy: decode_json::<Policy>(self.policy)?,
                graph: decode_json::<SourceGraph>(self.graph)?,
                staged_update: self
                    .staged_update
                    .map(decode_json::<StagedRepoUpdate>)
                    .transpose()?,
                memberships,
                invitations: decode_json::<Vec<RepoInvitation>>(self.invitations)?,
            })
        }
    }
}

pub(crate) mod membership {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repo_memberships")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: String,
        pub role: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(membership: &RepoMembership) -> Self {
            Self {
                repo_id: membership.repo_id.clone(),
                user_id: membership.user_id.clone(),
                role: encode_enum(membership.role).expect("RepoRole serializes to a string"),
            }
        }

        pub(crate) fn try_into_domain(self) -> Result<RepoMembership, ApiError> {
            Ok(RepoMembership {
                repo_id: self.repo_id,
                user_id: self.user_id,
                role: decode_enum::<RepoRole>(self.role)?,
            })
        }
    }
}

pub(crate) mod metadata_lock {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_metadata_locks")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub key: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[allow(dead_code)]
fn _keeps_invitation_state_serde_visible(state: InvitationState) -> InvitationState {
    state
}
