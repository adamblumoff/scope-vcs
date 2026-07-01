use super::metadata_schema::*;
use sea_orm::DbErr;
use sea_orm_migration::{manager::SchemaManager, prelude::*};

pub async fn ensure_outbox_tables(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(OutboxJobs::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(OutboxJobs::Id)
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(OutboxJobs::IdempotencyKey)
                        .string()
                        .not_null()
                        .unique_key(),
                )
                .col(ColumnDef::new(OutboxJobs::Kind).string().not_null())
                .col(ColumnDef::new(OutboxJobs::RepoId).string().not_null())
                .col(
                    ColumnDef::new(OutboxJobs::RepoVersion)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(OutboxJobs::Payload).json_binary().not_null())
                .col(ColumnDef::new(OutboxJobs::State).string().not_null())
                .col(
                    ColumnDef::new(OutboxJobs::Attempts)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(OutboxJobs::NextRunAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(OutboxJobs::LeaseOwner).string())
                .col(ColumnDef::new(OutboxJobs::LeaseExpiresAtUnix).big_integer())
                .col(ColumnDef::new(OutboxJobs::LastError).text())
                .col(
                    ColumnDef::new(OutboxJobs::CreatedAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(OutboxJobs::UpdatedAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(OutboxJobs::CompletedAtUnix).big_integer())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_outbox_jobs_repo")
                        .from(OutboxJobs::Table, OutboxJobs::RepoId)
                        .to(Repositories::Table, Repositories::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_scope_outbox_jobs_ready")
                .table(OutboxJobs::Table)
                .col(OutboxJobs::State)
                .col(OutboxJobs::NextRunAtUnix)
                .col(OutboxJobs::CreatedAtUnix)
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_scope_outbox_jobs_repo")
                .table(OutboxJobs::Table)
                .col(OutboxJobs::RepoId)
                .col(OutboxJobs::RepoVersion)
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    Ok(())
}
