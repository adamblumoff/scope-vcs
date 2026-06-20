use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "pending_import_files")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub repo_id: String,
    pub file_index: i32,
    pub path: String,
    pub mode: String,
    pub oid: String,
    pub blob_object_key: Option<String>,
    pub content_sha256: Option<String>,
    pub size_bytes: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::pending_imports::Entity",
        from = "Column::RepoId",
        to = "super::pending_imports::Column::RepoId"
    )]
    PendingImport,
}

impl Related<super::pending_imports::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::PendingImport.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
