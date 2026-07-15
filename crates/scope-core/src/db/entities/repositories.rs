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
            })
        }

        pub fn try_into_domain(
            self,
            facts: RepositoryFacts,
            members: Vec<RepositoryMember>,
            invitations: Vec<RepositoryInvite>,
            history: crate::db::history_rows::RepositoryHistory,
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
                graph: history.graph,
                visibility_events: history.visibility_events,
                live_files: history.live_files,
                git_head: facts.git_head,
                git_segments: facts.git_segments,
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
pub mod git_head {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_git_heads")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub head_oid: String,
        pub segment_sequence: i64,
        pub change_version: i64,
        pub manifest_object_key: String,
        pub manifest_sha256: String,
        pub manifest_size_bytes: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(repo_id: &str, head: &GitHead) -> Result<Self, ApiError> {
            Ok(Self {
                repo_id: repo_id.to_string(),
                head_oid: head.head_oid.clone(),
                segment_sequence: u64_to_i64(head.segment_sequence, "Git segment sequence")?,
                change_version: u64_to_i64(head.change_version, "Git head change version")?,
                manifest_object_key: head.manifest.object_key.clone(),
                manifest_sha256: head.manifest.sha256.clone(),
                manifest_size_bytes: u64_to_i64(head.manifest.size_bytes, "Git manifest size")?,
            })
        }

        pub fn try_into_domain(self) -> Result<GitHead, ApiError> {
            Ok(GitHead {
                head_oid: self.head_oid.clone(),
                segment_sequence: i64_to_u64(self.segment_sequence, "Git segment sequence")?,
                change_version: i64_to_u64(self.change_version, "Git head change version")?,
                manifest: SourceBlob {
                    object_key: self.manifest_object_key,
                    sha256: self.manifest_sha256,
                    git_oid: self.head_oid.clone(),
                    git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                    size_bytes: i64_to_u64(self.manifest_size_bytes, "Git manifest size")?,
                },
            })
        }
    }
}

pub mod git_segment {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_git_segments")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub sequence: i64,
        pub base_oid: Option<String>,
        pub head_oid: String,
        pub object_key: String,
        pub sha256: String,
        pub size_bytes: i64,
        pub manifest_object_key: String,
        pub manifest_sha256: String,
        pub manifest_size_bytes: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(repo_id: &str, segment: &GitSegment) -> Result<Self, ApiError> {
            Ok(Self {
                repo_id: repo_id.to_string(),
                sequence: u64_to_i64(segment.sequence, "Git segment sequence")?,
                base_oid: segment.base_oid.clone(),
                head_oid: segment.head_oid.clone(),
                object_key: segment.object.object_key.clone(),
                sha256: segment.object.sha256.clone(),
                size_bytes: u64_to_i64(segment.object.size_bytes, "Git segment size")?,
                manifest_object_key: segment.manifest.object_key.clone(),
                manifest_sha256: segment.manifest.sha256.clone(),
                manifest_size_bytes: u64_to_i64(
                    segment.manifest.size_bytes,
                    "Git segment manifest size",
                )?,
            })
        }

        pub fn try_into_domain(self) -> Result<GitSegment, ApiError> {
            Ok(GitSegment {
                sequence: i64_to_u64(self.sequence, "Git segment sequence")?,
                base_oid: self.base_oid,
                head_oid: self.head_oid.clone(),
                object: SourceBlob {
                    object_key: self.object_key,
                    sha256: self.sha256,
                    git_oid: self.head_oid.clone(),
                    git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                    size_bytes: i64_to_u64(self.size_bytes, "Git segment size")?,
                },
                manifest: SourceBlob {
                    object_key: self.manifest_object_key,
                    sha256: self.manifest_sha256,
                    git_oid: self.head_oid,
                    git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                    size_bytes: i64_to_u64(self.manifest_size_bytes, "Git segment manifest size")?,
                },
            })
        }
    }
}
