use super::metadata_schema::*;
use sea_orm::DbErr;
use sea_orm_migration::{manager::SchemaManager, prelude::*};

pub async fn ensure_read_model_tables(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(ProjectionReadModels::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(ProjectionReadModels::RepoId)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ProjectionReadModels::RepoVersion)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ProjectionReadModels::Source)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ProjectionReadModels::Audience)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ProjectionReadModels::RebuiltAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ProjectionReadModels::FileCount)
                        .big_integer()
                        .not_null(),
                )
                .primary_key(
                    Index::create()
                        .name("pk_scope_projection_read_models")
                        .col(ProjectionReadModels::RepoId)
                        .col(ProjectionReadModels::Source)
                        .col(ProjectionReadModels::Audience),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_projection_read_models_repo")
                        .from(ProjectionReadModels::Table, ProjectionReadModels::RepoId)
                        .to(Repositories::Table, Repositories::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_table(
            Table::create()
                .table(ProjectionFiles::Table)
                .if_not_exists()
                .col(ColumnDef::new(ProjectionFiles::RepoId).string().not_null())
                .col(
                    ColumnDef::new(ProjectionFiles::RepoVersion)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(ProjectionFiles::Source).string().not_null())
                .col(
                    ColumnDef::new(ProjectionFiles::Audience)
                        .string()
                        .not_null(),
                )
                .col(ColumnDef::new(ProjectionFiles::PathKey).string().not_null())
                .col(ColumnDef::new(ProjectionFiles::Path).string().not_null())
                .col(ColumnDef::new(ProjectionFiles::Oid).string().not_null())
                .col(
                    ColumnDef::new(ProjectionFiles::Visibility)
                        .string()
                        .not_null(),
                )
                .primary_key(
                    Index::create()
                        .name("pk_scope_projection_files")
                        .col(ProjectionFiles::RepoId)
                        .col(ProjectionFiles::Source)
                        .col(ProjectionFiles::Audience)
                        .col(ProjectionFiles::PathKey),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_projection_files_repo")
                        .from(ProjectionFiles::Table, ProjectionFiles::RepoId)
                        .to(Repositories::Table, Repositories::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_scope_projection_files_lookup")
                .table(ProjectionFiles::Table)
                .col(ProjectionFiles::RepoId)
                .col(ProjectionFiles::RepoVersion)
                .col(ProjectionFiles::Source)
                .col(ProjectionFiles::Audience)
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    Ok(())
}
