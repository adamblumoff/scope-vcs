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
            audience: &str,
            rebuilt_at_unix: u64,
            file_count: usize,
        ) -> Self {
            Self {
                repo_id: repo_id.to_string(),
                repo_version: u64_to_i64_saturating(repo_version),
                source: "live".to_string(),
                audience: audience.to_string(),
                rebuilt_at_unix: u64_to_i64_saturating(rebuilt_at_unix),
                file_count: usize_to_i64_saturating(file_count),
            }
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
    }

    fn projection_file_path_key(path: &ScopePath) -> String {
        format!("sha256:{:x}", Sha256::digest(path.as_str().as_bytes()))
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn live(
            repo_id: &str,
            repo_version: u64,
            audience: &str,
            file: ProjectionViewFile,
        ) -> Result<Self, ApiError> {
            let path_key = projection_file_path_key(&file.path);
            Ok(Self {
                repo_id: repo_id.to_string(),
                repo_version: u64_to_i64_saturating(repo_version),
                source: "live".to_string(),
                audience: audience.to_string(),
                path_key,
                path: file.path.as_str().to_string(),
                oid: file.oid,
                visibility: encode_enum(file.visibility)?,
            })
        }

        pub fn try_into_view(self) -> Result<ProjectionViewFile, ApiError> {
            Ok(ProjectionViewFile {
                path: ScopePath::parse(&self.path).map_err(ApiError::internal)?,
                oid: self.oid,
                tracked: true,
                visibility: decode_enum::<Visibility>(self.visibility)?,
            })
        }
    }
}
