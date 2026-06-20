use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "file_changes")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub repo_id: String,
    pub logical_commit_id: String,
    pub change_index: i32,
    pub path: String,
    pub old_blob_object_key: Option<String>,
    pub new_blob_object_key: Option<String>,
    pub old_content_sha256: Option<String>,
    pub new_content_sha256: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
