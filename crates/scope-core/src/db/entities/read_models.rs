use super::*;

pub mod projection_read_model {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_projection_read_models")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub repo_version: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub source: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub audience: String,
        pub rebuilt_at_unix: i64,
        pub file_count: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn live(
            repo_id: &str,
            repo_version: u64,
            audience: ProjectionAudience,
            rebuilt_at_unix: u64,
            file_count: usize,
        ) -> Result<Self, ApiError> {
            Ok(Self {
                repo_id: repo_id.to_string(),
                repo_version: u64_to_i64(repo_version, "projection repository version")?,
                source: ProjectionSource::Live.as_str().to_string(),
                audience: audience.as_str().to_string(),
                rebuilt_at_unix: u64_to_i64(rebuilt_at_unix, "projection rebuild time")?,
                file_count: usize_to_i64(file_count, "projection file count")?,
            })
        }
    }
}
pub mod projection_file {
    use super::*;
    use sha2::{Digest as _, Sha256};

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_projection_files")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub repo_version: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub source: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub audience: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub path_key: String,
        pub path: String,
        pub oid: String,
        pub visibility: String,
        pub object_key: String,
        pub sha256: String,
        pub size_bytes: i64,
        pub git_file_mode: String,
    }

    pub(crate) fn projection_file_path_key(path: &ScopePath) -> String {
        format!("sha256:{:x}", Sha256::digest(path.as_str().as_bytes()))
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn live(
            repo_id: &str,
            repo_version: u64,
            audience: ProjectionAudience,
            content: ProjectionViewFileContent,
        ) -> Result<Self, ApiError> {
            if !content.file.tracked {
                return Err(ApiError::internal_message(
                    "projection file content must be tracked",
                ));
            }
            if content.file.oid != content.blob.git_oid {
                return Err(ApiError::internal_message(
                    "projection file and blob Git OIDs must match",
                ));
            }
            if !is_supported_git_file_mode(&content.blob.git_file_mode) {
                return Err(ApiError::internal_message(
                    "projection file has unsupported Git mode",
                ));
            }
            let path_key = projection_file_path_key(&content.file.path);
            Ok(Self {
                repo_id: repo_id.to_string(),
                repo_version: u64_to_i64(repo_version, "projection repository version")?,
                source: ProjectionSource::Live.as_str().to_string(),
                audience: audience.as_str().to_string(),
                path_key,
                path: content.file.path.as_str().to_string(),
                oid: content.file.oid,
                visibility: encode_enum(content.file.visibility)?,
                object_key: content.blob.object_key,
                sha256: content.blob.sha256,
                size_bytes: u64_to_i64(content.blob.size_bytes, "projection file size")?,
                git_file_mode: content.blob.git_file_mode,
            })
        }

        pub fn try_into_content(self) -> Result<ProjectionViewFileContent, ApiError> {
            if !is_supported_git_file_mode(&self.git_file_mode) {
                return Err(ApiError::internal_message(
                    "projection file has unsupported Git mode",
                ));
            }
            Ok(ProjectionViewFileContent {
                file: ProjectionViewFile {
                    path: ScopePath::parse(&self.path).map_err(ApiError::internal)?,
                    oid: self.oid.clone(),
                    tracked: true,
                    visibility: decode_enum::<Visibility>(self.visibility)?,
                },
                blob: SourceBlob {
                    object_key: self.object_key,
                    sha256: self.sha256,
                    git_oid: self.oid,
                    git_file_mode: self.git_file_mode,
                    size_bytes: i64_to_u64(self.size_bytes, "projection file size")?,
                },
            })
        }

        pub fn try_into_view(self) -> Result<ProjectionViewFile, ApiError> {
            Ok(self.try_into_content()?.file)
        }
    }
}
