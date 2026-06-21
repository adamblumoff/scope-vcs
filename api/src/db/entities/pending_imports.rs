use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "pending_imports")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub repo_id: String,
    pub default_branch: String,
    pub head_oid: String,
    pub tree_oid: String,
    pub imported_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::repositories::Entity",
        from = "Column::RepoId",
        to = "super::repositories::Column::Id"
    )]
    Repository,
}

impl Related<super::repositories::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Repository.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
