use super::*;

pub mod repository {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repositories")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub incarnation_id: String,
        pub owner_handle: String,
        pub name: String,
        pub owner_user_id: String,
        pub publication_state: String,
        pub change_version: i64,
        pub repo_config: Json,
        pub policy: Json,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(repo: &Repository) -> Result<Self, PostgresError> {
            Ok(Self {
                id: repo.record.id.clone(),
                incarnation_id: repo.record.incarnation_id.clone(),
                owner_handle: repo.record.owner_handle.clone(),
                name: repo.record.name.clone(),
                owner_user_id: repo.record.owner_user_id.clone(),
                publication_state: encode_enum(repo.record.lifecycle_state)?,
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
        ) -> Result<Repository, PostgresError> {
            let lifecycle_state = decode_enum::<RepoLifecycleState>(self.publication_state)?;
            Ok(Repository {
                record: RepoRecord {
                    id: self.id.clone(),
                    incarnation_id: self.incarnation_id,
                    owner_handle: self.owner_handle,
                    name: self.name,
                    owner_user_id: self.owner_user_id,
                    lifecycle_state,
                    change_version: i64_to_u64(self.change_version, "repository change version")?,
                },
                repo_config: decode_json(self.repo_config)?,
                first_push_token: facts.first_push_token,
                git_push_token: facts.git_push_token,
                policy: decode_json::<Policy>(self.policy)?,
                graph: history.graph,
                visibility_change_sets: history.visibility_change_sets,
                live_files: history.live_files,
                git_head: facts.git_head,
                git_pack_spans: facts.git_pack_spans,
                members,
                invitations,
            })
        }
    }
}

pub mod repository_landing_file {
    use super::*;
    use scope_domain::landing_file::{REPOSITORY_LANDING_FILE_PATH, RepositoryLandingFile};

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_landing_files")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub path: String,
        pub oid: String,
        pub sha256: String,
        pub size_bytes: i64,
        pub git_file_mode: String,
        pub content_bytes: Vec<u8>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(
            repo_id: &str,
            landing_file: RepositoryLandingFile,
        ) -> Result<Self, PostgresError> {
            landing_file
                .validate_integrity()
                .map_err(PostgresError::internal)?;
            Ok(Self {
                repo_id: repo_id.to_string(),
                path: REPOSITORY_LANDING_FILE_PATH.to_string(),
                oid: landing_file.oid,
                sha256: landing_file.sha256,
                size_bytes: u64_to_i64(landing_file.size_bytes, "repository landing file size")?,
                git_file_mode: landing_file.git_file_mode,
                content_bytes: landing_file.content_bytes,
            })
        }

        pub fn try_into_domain(self) -> Result<RepositoryLandingFile, PostgresError> {
            if self.path != REPOSITORY_LANDING_FILE_PATH {
                return Err(PostgresError::internal_message(
                    "repository landing file has an unexpected path",
                ));
            }
            let size_bytes = i64_to_u64(self.size_bytes, "repository landing file size")?;
            let landing_file = RepositoryLandingFile {
                oid: self.oid,
                sha256: self.sha256,
                size_bytes,
                git_file_mode: self.git_file_mode,
                content_bytes: self.content_bytes,
            };
            landing_file
                .validate_integrity()
                .map_err(PostgresError::internal)?;
            Ok(landing_file)
        }
    }
}

pub mod repository_workflow_catalog {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_workflow_catalogs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub source_head_oid: String,
        pub source_change_version: i64,
        pub configuration_error: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_workflow_file {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_workflow_files")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub path: String,
        pub oid: String,
        pub size_bytes: i64,
        pub git_file_mode: String,
        pub content_bytes: Vec<u8>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
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
        pub fn from_domain(repo_id: &str, token: &FirstPushToken) -> Result<Self, PostgresError> {
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

        pub fn try_into_domain(self) -> Result<FirstPushToken, PostgresError> {
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
        pub fn from_domain(repo_id: &str, token: &GitPushToken) -> Result<Self, PostgresError> {
            Ok(Self {
                repo_id: repo_id.to_string(),
                token_hash: token.token_hash.clone(),
                owner_user_id: token.owner_user_id.clone(),
                created_at_unix: u64_to_i64(token.created_at_unix, "Git push token creation time")?,
            })
        }

        pub fn try_into_domain(self) -> Result<GitPushToken, PostgresError> {
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
        pub push_sequence: i64,
        pub change_version: i64,
        pub manifest_object_key: String,
        pub manifest_sha256: String,
        pub manifest_size_bytes: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(repo_id: &str, head: &GitHead) -> Result<Self, PostgresError> {
            Ok(Self {
                repo_id: repo_id.to_string(),
                head_oid: head.head_oid.clone(),
                push_sequence: u64_to_i64(head.push_sequence, "Git push sequence")?,
                change_version: u64_to_i64(head.change_version, "Git head change version")?,
                manifest_object_key: serde_json::to_string(&head.manifest.content_ref)
                    .map_err(PostgresError::internal)?,
                manifest_sha256: head.manifest.sha256.clone(),
                manifest_size_bytes: u64_to_i64(head.manifest.size_bytes, "Git manifest size")?,
            })
        }

        pub fn try_into_domain(self) -> Result<GitHead, PostgresError> {
            Ok(GitHead {
                head_oid: self.head_oid.clone(),
                push_sequence: i64_to_u64(self.push_sequence, "Git push sequence")?,
                change_version: i64_to_u64(self.change_version, "Git head change version")?,
                manifest: SourceBlob {
                    content_ref: serde_json::from_str(&self.manifest_object_key)
                        .map_err(PostgresError::internal)?,
                    sha256: self.manifest_sha256,
                    git_oid: self.head_oid.clone(),
                    git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                    size_bytes: i64_to_u64(self.manifest_size_bytes, "Git manifest size")?,
                },
            })
        }
    }
}

pub mod git_pack_span {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_git_segments")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub first_sequence: i64,
        pub last_sequence: i64,
        pub geometric_tier: i32,
        pub base_oid: Option<String>,
        pub head_oid: String,
        pub segment_id: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(repo_id: &str, span: &GitPackSpan) -> Result<Self, PostgresError> {
            Ok(Self {
                repo_id: repo_id.to_string(),
                first_sequence: u64_to_i64(span.first_sequence, "Git pack span first sequence")?,
                last_sequence: u64_to_i64(span.last_sequence, "Git pack span last sequence")?,
                geometric_tier: u32_to_i32(span.geometric_tier, "Git pack span geometric tier")?,
                base_oid: span.base_oid.clone(),
                head_oid: span.head_oid.clone(),
                segment_id: span.segment.segment_id.clone(),
            })
        }

        pub fn try_into_domain(self, segment: GitSegmentRef) -> Result<GitPackSpan, PostgresError> {
            if self.segment_id != segment.segment_id {
                return Err(PostgresError::internal_message(
                    "Git pack span resolved the wrong segment",
                ));
            }
            let span = GitPackSpan {
                first_sequence: i64_to_u64(self.first_sequence, "Git pack span first sequence")?,
                last_sequence: i64_to_u64(self.last_sequence, "Git pack span last sequence")?,
                geometric_tier: i32_to_u32(self.geometric_tier, "Git pack span geometric tier")?,
                base_oid: self.base_oid,
                head_oid: self.head_oid.clone(),
                segment,
            };
            let expected_tier = span
                .expected_geometric_tier()
                .map_err(|error| PostgresError::internal_message(error.to_string()))?;
            if span.geometric_tier != expected_tier {
                return Err(PostgresError::internal_message(format!(
                    "Git pack span {}..{} has geometric tier {}, expected {expected_tier}",
                    span.first_sequence, span.last_sequence, span.geometric_tier
                )));
            }
            Ok(span)
        }
    }
}

pub mod git_segment_upload {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_git_segment_uploads")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub segment_id: String,
        pub repo_id: String,
        pub object_key: String,
        pub state: String,
        pub sha256: Option<String>,
        pub plaintext_bytes: Option<i64>,
        pub encrypted_bytes: Option<i64>,
        pub encoding_version: i32,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        #[cfg(any(
            test,
            feature = "local-dev",
            feature = "smoke-seed",
            feature = "test-support"
        ))]
        pub fn from_domain(upload: &GitSegmentUpload) -> Result<Self, PostgresError> {
            Ok(Self {
                segment_id: upload.segment_id.clone(),
                repo_id: upload.repository_id.clone(),
                object_key: upload.object_key.clone(),
                state: encode_enum(upload.state)?,
                sha256: upload.sha256.clone(),
                plaintext_bytes: upload
                    .plaintext_bytes
                    .map(|value| u64_to_i64(value, "Git segment plaintext size"))
                    .transpose()?,
                encrypted_bytes: upload
                    .encrypted_bytes
                    .map(|value| u64_to_i64(value, "Git segment encrypted size"))
                    .transpose()?,
                encoding_version: u32_to_i32(
                    upload.encoding_version,
                    "Git segment encoding version",
                )?,
                created_at_unix: u64_to_i64(
                    upload.created_at_unix,
                    "Git segment upload creation time",
                )?,
                updated_at_unix: u64_to_i64(
                    upload.updated_at_unix,
                    "Git segment upload update time",
                )?,
            })
        }

        pub fn try_into_domain(self) -> Result<GitSegmentUpload, PostgresError> {
            Ok(GitSegmentUpload {
                segment_id: self.segment_id,
                repository_id: self.repo_id,
                object_key: self.object_key,
                state: decode_enum(self.state)?,
                sha256: self.sha256,
                plaintext_bytes: self
                    .plaintext_bytes
                    .map(|value| i64_to_u64(value, "Git segment plaintext size"))
                    .transpose()?,
                encrypted_bytes: self
                    .encrypted_bytes
                    .map(|value| i64_to_u64(value, "Git segment encrypted size"))
                    .transpose()?,
                encoding_version: i32_to_u32(
                    self.encoding_version,
                    "Git segment encoding version",
                )?,
                created_at_unix: i64_to_u64(
                    self.created_at_unix,
                    "Git segment upload creation time",
                )?,
                updated_at_unix: i64_to_u64(
                    self.updated_at_unix,
                    "Git segment upload update time",
                )?,
            })
        }

        pub fn ready_segment_ref(&self) -> Result<GitSegmentRef, PostgresError> {
            let state = decode_enum::<GitSegmentUploadState>(self.state.clone())?;
            if !matches!(
                state,
                GitSegmentUploadState::Ready
                    | GitSegmentUploadState::Published
                    | GitSegmentUploadState::Retained
            ) {
                return Err(PostgresError::internal_message(format!(
                    "Git segment {} is not ready for repository publication",
                    self.segment_id
                )));
            }
            Ok(GitSegmentRef {
                segment_id: self.segment_id.clone(),
                sha256: self.sha256.clone().ok_or_else(|| {
                    PostgresError::internal_message("ready Git segment has no SHA-256 digest")
                })?,
                plaintext_bytes: i64_to_u64(
                    self.plaintext_bytes.ok_or_else(|| {
                        PostgresError::internal_message("ready Git segment has no plaintext size")
                    })?,
                    "Git segment plaintext size",
                )?,
                encoding_version: i32_to_u32(
                    self.encoding_version,
                    "Git segment encoding version",
                )?,
            })
        }
    }
}
