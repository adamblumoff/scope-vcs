use super::metadata_schema::*;
use sea_orm::DbErr;
use sea_orm_migration::{manager::SchemaManager, prelude::*};

pub async fn ensure_repository_collaboration_tables(
    manager: &SchemaManager<'_>,
) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(RepositoryMembers::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(RepositoryMembers::RepoId)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(RepositoryMembers::UserId)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(RepositoryMembers::Permissions)
                        .json_binary()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(RepositoryMembers::CreatedAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(RepositoryMembers::UpdatedAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .primary_key(
                    Index::create()
                        .name("pk_scope_repository_members")
                        .col(RepositoryMembers::RepoId)
                        .col(RepositoryMembers::UserId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_repository_members_repo")
                        .from(RepositoryMembers::Table, RepositoryMembers::RepoId)
                        .to(Repositories::Table, Repositories::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_repository_members_user")
                        .from(RepositoryMembers::Table, RepositoryMembers::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_scope_repository_members_user")
                .table(RepositoryMembers::Table)
                .col(RepositoryMembers::UserId)
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    manager
        .create_table(
            Table::create()
                .table(RepositoryInvites::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(RepositoryInvites::Id)
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(RepositoryInvites::RepoId)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(RepositoryInvites::InvitedEmail)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(RepositoryInvites::InvitedEmailNormalized)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(RepositoryInvites::Permissions)
                        .json_binary()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(RepositoryInvites::InvitedByUserId)
                        .string()
                        .not_null(),
                )
                .col(ColumnDef::new(RepositoryInvites::State).string().not_null())
                .col(
                    ColumnDef::new(RepositoryInvites::TokenHash)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(RepositoryInvites::CreatedAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(RepositoryInvites::UpdatedAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(RepositoryInvites::ExpiresAtUnix)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(RepositoryInvites::AcceptedByUserId).string())
                .col(ColumnDef::new(RepositoryInvites::AcceptedAtUnix).big_integer())
                .col(ColumnDef::new(RepositoryInvites::RevokedAtUnix).big_integer())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_repository_invites_repo")
                        .from(RepositoryInvites::Table, RepositoryInvites::RepoId)
                        .to(Repositories::Table, Repositories::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_repository_invites_inviter")
                        .from(RepositoryInvites::Table, RepositoryInvites::InvitedByUserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_repository_invites_accepted_user")
                        .from(
                            RepositoryInvites::Table,
                            RepositoryInvites::AcceptedByUserId,
                        )
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_scope_repository_invites_repo_email")
                .table(RepositoryInvites::Table)
                .col(RepositoryInvites::RepoId)
                .col(RepositoryInvites::InvitedEmailNormalized)
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_scope_repository_invites_token_hash")
                .table(RepositoryInvites::Table)
                .col(RepositoryInvites::TokenHash)
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    Ok(())
}
