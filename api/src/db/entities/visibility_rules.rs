use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "visibility_rules")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub repo_id: String,
    pub path: String,
    pub visibility: String,
    pub allowed_principal_ids: Json,
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
