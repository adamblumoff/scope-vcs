use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "logical_commits")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub repo_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub sequence: i32,
    pub parent_ids: Json,
    pub author_id: String,
    pub author_visibility: String,
    pub message: String,
    pub mixed_policy: String,
    pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::repositories::Entity",
        from = "Column::RepoId",
        to = "super::repositories::Column::Id"
    )]
    Repository,
    #[sea_orm(
        belongs_to = "super::users::Entity",
        from = "Column::AuthorId",
        to = "super::users::Column::Id"
    )]
    Author,
}

impl Related<super::repositories::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Repository.def()
    }
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Author.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
