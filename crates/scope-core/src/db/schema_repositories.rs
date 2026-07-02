use super::metadata_schema::*;
use sea_orm::DbErr;
use sea_orm_migration::{manager::SchemaManager, prelude::*};

pub async fn ensure_repository_tables(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Repositories::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Repositories::Id)
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Repositories::OwnerHandle)
                        .string()
                        .not_null(),
                )
                .col(ColumnDef::new(Repositories::Name).string().not_null())
                .col(
                    ColumnDef::new(Repositories::OwnerUserId)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Repositories::PublicationState)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Repositories::DefaultVisibility)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Repositories::ChangeVersion)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(Repositories::PendingImport).json_binary())
                .col(
                    ColumnDef::new(Repositories::Policy)
                        .json_binary()
                        .not_null(),
                )
                .col(ColumnDef::new(Repositories::Graph).json_binary().not_null())
                .col(
                    ColumnDef::new(Repositories::VisibilityEvents)
                        .json_binary()
                        .not_null(),
                )
                .col(ColumnDef::new(Repositories::StagedUpdate).json_binary())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_repositories_owner")
                        .from(Repositories::Table, Repositories::OwnerUserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_scope_repositories_owner_name")
                .table(Repositories::Table)
                .col(Repositories::OwnerHandle)
                .col(Repositories::Name)
                .unique()
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    Ok(())
}
