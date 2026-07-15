use super::*;

pub mod logical_commit {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_logical_commits")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub ordinal: i64,
        pub parent_ids: Json,
        pub author_id: String,
        pub author_visibility: String,
        pub message: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod file_change {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_file_changes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub commit_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub ordinal: i64,
        pub path: String,
        pub old_content: Option<Json>,
        pub new_content: Option<Json>,
        pub visibility: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod visibility_event {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_visibility_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub ordinal: i64,
        pub after_commit_id: Option<String>,
        pub source_commit_id: Option<String>,
        pub author_id: String,
        pub path: String,
        pub old_visibility: String,
        pub new_visibility: String,
        pub current_content: Option<Json>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod live_file {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_live_files")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub path: String,
        pub content: Json,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod object_reference {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_object_references")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub object_key: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub ref_kind: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub ref_id: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
