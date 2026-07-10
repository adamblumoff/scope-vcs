use super::*;

pub mod outbox_job {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_outbox_jobs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        #[sea_orm(unique)]
        pub idempotency_key: String,
        pub kind: String,
        pub repo_id: String,
        pub repo_version: i64,
        pub payload: Json,
        pub state: String,
        pub attempts: i64,
        pub next_run_at_unix: i64,
        pub lease_owner: Option<String>,
        pub lease_expires_at_unix: Option<i64>,
        pub last_error: Option<String>,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
        pub completed_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn projection_read_model_rebuild(
            id: String,
            repo: &StoredRepository,
            now: u64,
        ) -> Result<Self, ApiError> {
            let repo_version = u64_to_i64(repo.record.change_version, "repository change version")?;
            Ok(Self {
                id,
                idempotency_key: projection_read_model_rebuild_idempotency_key(
                    &repo.record.id,
                    repo.record.change_version,
                ),
                kind: "projection_read_model_rebuild".to_string(),
                repo_id: repo.record.id.clone(),
                repo_version,
                payload: encode_json(&serde_json::json!({
                    "repo_id": repo.record.id.clone(),
                    "repo_version": repo.record.change_version,
                    "source": ProjectionSource::Live.as_str(),
                }))?,
                state: "ready".to_string(),
                attempts: 0,
                next_run_at_unix: u64_to_i64(now, "outbox next run time")?,
                lease_owner: None,
                lease_expires_at_unix: None,
                last_error: None,
                created_at_unix: u64_to_i64(now, "outbox creation time")?,
                updated_at_unix: u64_to_i64(now, "outbox update time")?,
                completed_at_unix: None,
            })
        }
    }

    pub fn projection_read_model_rebuild_idempotency_key(
        repo_id: &str,
        repo_version: u64,
    ) -> String {
        format!("projection_read_model_rebuild:{repo_id}:{repo_version}")
    }
}
pub mod metadata_lock {
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
pub mod repo_storage_cleanup_job {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repo_storage_cleanup_jobs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub generation: String,
        pub owner_handle: String,
        pub repo_name: String,
        pub attempts: i32,
        pub next_run_at_unix: i64,
        pub last_error: Option<String>,
        pub completed_at_unix: Option<i64>,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(
            cleanup: &RepoStorageCleanup,
            generation: String,
            now_unix: u64,
        ) -> Result<Self, ApiError> {
            let repo_id = crate::domain::store::repo_id(&cleanup.owner_handle, &cleanup.repo_name);
            let now_unix = u64_to_i64(now_unix, "cleanup creation time")?;
            Ok(Self {
                repo_id,
                generation,
                owner_handle: cleanup.owner_handle.clone(),
                repo_name: cleanup.repo_name.clone(),
                attempts: 0,
                next_run_at_unix: now_unix,
                last_error: None,
                completed_at_unix: None,
                created_at_unix: now_unix,
                updated_at_unix: now_unix,
            })
        }

        pub fn into_domain(self) -> RepoStorageCleanup {
            RepoStorageCleanup {
                owner_handle: self.owner_handle,
                repo_name: self.repo_name,
            }
        }
    }
}
pub mod source_blob_cleanup_job {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_source_blob_cleanup_jobs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub object_key: String,
        pub generation: String,
        pub sha256: String,
        pub git_oid: String,
        pub size_bytes: i64,
        pub attempts: i32,
        pub next_run_at_unix: i64,
        pub last_error: Option<String>,
        pub completed_at_unix: Option<i64>,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(
            blob: &SourceBlob,
            generation: String,
            now_unix: u64,
        ) -> Result<Self, ApiError> {
            let now_unix = u64_to_i64(now_unix, "cleanup creation time")?;
            Ok(Self {
                object_key: blob.object_key.clone(),
                generation,
                sha256: blob.sha256.clone(),
                git_oid: blob.git_oid.clone(),
                size_bytes: u64_to_i64(blob.size_bytes, "source blob size")?,
                attempts: 0,
                next_run_at_unix: now_unix,
                last_error: None,
                completed_at_unix: None,
                created_at_unix: now_unix,
                updated_at_unix: now_unix,
            })
        }

        pub fn try_into_domain(self) -> Result<SourceBlob, ApiError> {
            Ok(SourceBlob {
                object_key: self.object_key,
                sha256: self.sha256,
                git_oid: self.git_oid,
                git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                size_bytes: i64_to_u64(self.size_bytes, "source blob size")?,
            })
        }
    }
}
pub mod metadata_reset_event {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_metadata_reset_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub reset_at_unix: i64,
        pub trigger: String,
        pub reason: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
