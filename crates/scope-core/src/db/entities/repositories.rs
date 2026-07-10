use super::*;

pub mod repository {
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
        pub change_version: i64,
        pub repo_config: Json,
        pub policy: Json,
        pub graph: Json,
        pub visibility_events: Json,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(repo: &StoredRepository) -> Result<Self, ApiError> {
            Ok(Self {
                id: repo.record.id.clone(),
                owner_handle: repo.record.owner_handle.clone(),
                name: repo.record.name.clone(),
                owner_user_id: repo.record.owner_user_id.clone(),
                publication_state: encode_enum(repo.record.publication_state)?,
                default_visibility: encode_enum(repo.record.default_visibility)?,
                change_version: u64_to_i64(
                    repo.record.change_version,
                    "repository change version",
                )?,
                repo_config: encode_json(&repo.repo_config)?,
                policy: encode_json(&repo.policy)?,
                graph: encode_json(&repo.graph)?,
                visibility_events: encode_json(&repo.visibility_events)?,
            })
        }

        pub fn try_into_domain(
            self,
            facts: RepositoryFacts,
            members: Vec<RepositoryMember>,
            invitations: Vec<RepositoryInvite>,
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
                    change_version: i64_to_u64(self.change_version, "repository change version")?,
                },
                repo_config: decode_json(self.repo_config)?,
                first_push_token: facts.first_push_token,
                git_push_token: facts.git_push_token,
                policy: decode_json::<Policy>(self.policy)?,
                graph: decode_json::<SourceGraph>(self.graph)?,
                visibility_events: decode_json(self.visibility_events)?,
                git_snapshot: facts.git_snapshot,
                members,
                invitations,
            })
        }
    }
}
pub mod repository_first_push_token {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(table_name = "scope_repository_first_push_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub token_hash: String,
        pub owner_user_id: String,
        pub created_at_unix: i64,
        pub expires_at_unix: i64,
        pub used_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(repo_id: &str, token: &FirstPushToken) -> Result<Self, ApiError> {
            Ok(Self {
                repo_id: repo_id.to_string(),
                token_hash: token.token_hash.clone(),
                owner_user_id: token.owner_user_id.clone(),
                created_at_unix: u64_to_i64(
                    token.created_at_unix,
                    "first-push token creation time",
                )?,
                expires_at_unix: u64_to_i64(token.expires_at_unix, "first-push token expiry time")?,
                used_at_unix: token
                    .used_at_unix
                    .map(|value| u64_to_i64(value, "first-push token use time"))
                    .transpose()?,
            })
        }

        pub fn try_into_domain(self) -> Result<FirstPushToken, ApiError> {
            Ok(FirstPushToken {
                token_hash: self.token_hash,
                secret: None,
                owner_user_id: self.owner_user_id,
                created_at_unix: i64_to_u64(
                    self.created_at_unix,
                    "first-push token creation time",
                )?,
                expires_at_unix: i64_to_u64(self.expires_at_unix, "first-push token expiry time")?,
                used_at_unix: self
                    .used_at_unix
                    .map(|value| i64_to_u64(value, "first-push token use time"))
                    .transpose()?,
            })
        }
    }
}
pub mod repository_git_push_token {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_git_push_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub token_hash: String,
        pub owner_user_id: String,
        pub created_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(repo_id: &str, token: &GitPushToken) -> Result<Self, ApiError> {
            Ok(Self {
                repo_id: repo_id.to_string(),
                token_hash: token.token_hash.clone(),
                owner_user_id: token.owner_user_id.clone(),
                created_at_unix: u64_to_i64(token.created_at_unix, "Git push token creation time")?,
            })
        }

        pub fn try_into_domain(self) -> Result<GitPushToken, ApiError> {
            Ok(GitPushToken {
                token_hash: self.token_hash,
                owner_user_id: self.owner_user_id,
                created_at_unix: i64_to_u64(self.created_at_unix, "Git push token creation time")?,
            })
        }
    }
}
pub mod repository_git_snapshot {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_git_snapshots")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub object_key: String,
        pub sha256: String,
        pub git_oid: String,
        pub size_bytes: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(repo_id: &str, blob: &SourceBlob) -> Result<Self, ApiError> {
            Ok(Self {
                repo_id: repo_id.to_string(),
                object_key: blob.object_key.clone(),
                sha256: blob.sha256.clone(),
                git_oid: blob.git_oid.clone(),
                size_bytes: u64_to_i64(blob.size_bytes, "Git snapshot size")?,
            })
        }

        pub fn try_into_domain(self) -> Result<SourceBlob, ApiError> {
            Ok(SourceBlob {
                object_key: self.object_key,
                sha256: self.sha256,
                git_oid: self.git_oid,
                git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                size_bytes: i64_to_u64(self.size_bytes, "Git snapshot size")?,
            })
        }
    }
}
