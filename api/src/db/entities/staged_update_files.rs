use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "staged_update_files")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub staged_update_id: String,
    pub file_index: i32,
    pub path: String,
    pub old_blob_object_key: Option<String>,
    pub new_blob_object_key: Option<String>,
    pub visibility: String,
    pub change_kind: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::staged_updates::Entity",
        from = "Column::StagedUpdateId",
        to = "super::staged_updates::Column::Id"
    )]
    StagedUpdate,
}

impl Related<super::staged_updates::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::StagedUpdate.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
