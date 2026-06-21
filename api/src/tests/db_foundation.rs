use crate::db::{Migrator, entities};
use sea_orm::EntityName;
use sea_orm_migration::MigratorTrait;

#[test]
fn initial_migration_is_registered() {
    let migrations = Migrator::migrations();

    assert_eq!(migrations.len(), 1);
    assert_eq!(
        migrations[0].name(),
        "m20260620_000001_initial_metadata_graph"
    );
}

#[test]
fn canonical_entity_table_names_match_schema() {
    let table_names = [
        entities::users::Entity.table_name(),
        entities::repositories::Entity.table_name(),
        entities::repo_memberships::Entity.table_name(),
        entities::first_push_tokens::Entity.table_name(),
        entities::git_push_tokens::Entity.table_name(),
        entities::repo_settings::Entity.table_name(),
        entities::visibility_rules::Entity.table_name(),
        entities::logical_commits::Entity.table_name(),
        entities::file_changes::Entity.table_name(),
        entities::pending_imports::Entity.table_name(),
        entities::pending_import_files::Entity.table_name(),
        entities::staged_updates::Entity.table_name(),
        entities::staged_update_files::Entity.table_name(),
    ];

    assert_eq!(
        table_names,
        [
            "users",
            "repositories",
            "repo_memberships",
            "first_push_tokens",
            "git_push_tokens",
            "repo_settings",
            "visibility_rules",
            "logical_commits",
            "file_changes",
            "pending_imports",
            "pending_import_files",
            "staged_updates",
            "staged_update_files",
        ]
    );
}
