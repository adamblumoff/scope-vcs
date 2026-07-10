use super::*;

pub mod repository_member {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_members")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: String,
        pub permissions: Json,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(member: &RepositoryMember) -> Result<Self, ApiError> {
            Ok(Self {
                repo_id: member.repo_id.clone(),
                user_id: member.user_id.clone(),
                permissions: encode_json(&member.permissions)?,
                created_at_unix: u64_to_i64(
                    member.created_at_unix,
                    "repository member creation time",
                )?,
                updated_at_unix: u64_to_i64(
                    member.updated_at_unix,
                    "repository member update time",
                )?,
            })
        }

        pub fn try_into_domain(self) -> Result<RepositoryMember, ApiError> {
            Ok(RepositoryMember {
                repo_id: self.repo_id,
                user_id: self.user_id,
                permissions: decode_json::<RepositoryMemberPermissions>(self.permissions)?,
                created_at_unix: i64_to_u64(
                    self.created_at_unix,
                    "repository member creation time",
                )?,
                updated_at_unix: i64_to_u64(self.updated_at_unix, "repository member update time")?,
            })
        }
    }
}
pub mod repository_invite {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_invites")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub repo_id: String,
        pub invited_email: String,
        pub invited_email_normalized: String,
        pub permissions: Json,
        pub invited_by_user_id: String,
        pub state: String,
        pub token_hash: String,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
        pub expires_at_unix: i64,
        pub accepted_by_user_id: Option<String>,
        pub accepted_at_unix: Option<i64>,
        pub revoked_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(invite: &RepositoryInvite) -> Result<Self, ApiError> {
            Ok(Self {
                id: invite.id.clone(),
                repo_id: invite.repo_id.clone(),
                invited_email: invite.invited_email.clone(),
                invited_email_normalized: invite.invited_email_normalized.clone(),
                permissions: encode_json(&invite.permissions)?,
                invited_by_user_id: invite.invited_by_user_id.clone(),
                state: encode_enum(invite.state)?,
                token_hash: invite.token_hash.clone(),
                created_at_unix: u64_to_i64(
                    invite.created_at_unix,
                    "repository invite creation time",
                )?,
                updated_at_unix: u64_to_i64(
                    invite.updated_at_unix,
                    "repository invite update time",
                )?,
                expires_at_unix: u64_to_i64(
                    invite.expires_at_unix,
                    "repository invite expiry time",
                )?,
                accepted_by_user_id: invite.accepted_by_user_id.clone(),
                accepted_at_unix: invite
                    .accepted_at_unix
                    .map(|value| u64_to_i64(value, "repository invite acceptance time"))
                    .transpose()?,
                revoked_at_unix: invite
                    .revoked_at_unix
                    .map(|value| u64_to_i64(value, "repository invite revocation time"))
                    .transpose()?,
            })
        }

        pub fn try_into_domain(self) -> Result<RepositoryInvite, ApiError> {
            Ok(RepositoryInvite {
                id: self.id,
                repo_id: self.repo_id,
                invited_email: self.invited_email,
                invited_email_normalized: self.invited_email_normalized,
                permissions: decode_json::<RepositoryMemberPermissions>(self.permissions)?,
                invited_by_user_id: self.invited_by_user_id,
                state: decode_enum::<RepositoryInviteState>(self.state)?,
                token_hash: self.token_hash,
                created_at_unix: i64_to_u64(
                    self.created_at_unix,
                    "repository invite creation time",
                )?,
                updated_at_unix: i64_to_u64(self.updated_at_unix, "repository invite update time")?,
                expires_at_unix: i64_to_u64(self.expires_at_unix, "repository invite expiry time")?,
                accepted_by_user_id: self.accepted_by_user_id,
                accepted_at_unix: self
                    .accepted_at_unix
                    .map(|value| i64_to_u64(value, "repository invite acceptance time"))
                    .transpose()?,
                revoked_at_unix: self
                    .revoked_at_unix
                    .map(|value| i64_to_u64(value, "repository invite revocation time"))
                    .transpose()?,
            })
        }
    }
}
