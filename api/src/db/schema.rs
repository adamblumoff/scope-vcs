use sea_orm::{DatabaseConnection, DbErr};
use sea_orm_migration::{manager::SchemaManager, prelude::*};

pub(crate) async fn migrate_metadata_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    let manager = SchemaManager::new(db);
    manager
        .create_table(
            Table::create()
                .table(MetadataLocks::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(MetadataLocks::Key)
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(MetadataLocks::PendingRepoStorageDeletions)
                        .json_binary()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(MetadataLocks::PendingSourceBlobDeletions)
                        .json_binary()
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
    if !manager
        .has_column("scope_metadata_locks", "pending_source_blob_deletions")
        .await?
    {
        manager
            .alter_table(
                Table::alter()
                    .table(MetadataLocks::Table)
                    .add_column(
                        ColumnDef::new(MetadataLocks::PendingSourceBlobDeletions)
                            .json_binary()
                            .not_null()
                            .default("[]"),
                    )
                    .to_owned(),
            )
            .await?;
    }
    if !manager
        .has_column("scope_metadata_locks", "pending_repo_storage_deletions")
        .await?
    {
        manager
            .alter_table(
                Table::alter()
                    .table(MetadataLocks::Table)
                    .add_column(
                        ColumnDef::new(MetadataLocks::PendingRepoStorageDeletions)
                            .json_binary()
                            .not_null()
                            .default("[]"),
                    )
                    .to_owned(),
            )
            .await?;
    }

    manager
        .create_table(
            Table::create()
                .table(Users::Table)
                .if_not_exists()
                .col(ColumnDef::new(Users::Id).string().not_null().primary_key())
                .col(
                    ColumnDef::new(Users::Handle)
                        .string()
                        .not_null()
                        .unique_key(),
                )
                .col(ColumnDef::new(Users::Email).string().not_null())
                .col(ColumnDef::new(Users::EmailVerified).boolean().not_null())
                .col(ColumnDef::new(Users::Access).string().not_null())
                .to_owned(),
        )
        .await?;

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
                    ColumnDef::new(Repositories::Settings)
                        .json_binary()
                        .not_null(),
                )
                .col(ColumnDef::new(Repositories::FirstPushToken).json_binary())
                .col(ColumnDef::new(Repositories::GitPushToken).json_binary())
                .col(ColumnDef::new(Repositories::PendingImport).json_binary())
                .col(
                    ColumnDef::new(Repositories::Policy)
                        .json_binary()
                        .not_null(),
                )
                .col(ColumnDef::new(Repositories::Graph).json_binary().not_null())
                .col(ColumnDef::new(Repositories::GitSnapshot).json_binary())
                .col(ColumnDef::new(Repositories::StagedUpdate).json_binary())
                .col(
                    ColumnDef::new(Repositories::Invitations)
                        .json_binary()
                        .not_null(),
                )
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
    if !manager
        .has_column("scope_repositories", "git_snapshot")
        .await?
    {
        manager
            .alter_table(
                Table::alter()
                    .table(Repositories::Table)
                    .add_column(ColumnDef::new(Repositories::GitSnapshot).json_binary())
                    .to_owned(),
            )
            .await?;
    }

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

    manager
        .create_table(
            Table::create()
                .table(Memberships::Table)
                .if_not_exists()
                .col(ColumnDef::new(Memberships::RepoId).string().not_null())
                .col(ColumnDef::new(Memberships::UserId).string().not_null())
                .col(ColumnDef::new(Memberships::Role).string().not_null())
                .primary_key(
                    Index::create()
                        .name("pk_scope_repo_memberships")
                        .col(Memberships::RepoId)
                        .col(Memberships::UserId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_repo_memberships_repo")
                        .from(Memberships::Table, Memberships::RepoId)
                        .to(Repositories::Table, Repositories::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_scope_repo_memberships_user")
                        .from(Memberships::Table, Memberships::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_scope_repo_memberships_user")
                .table(Memberships::Table)
                .col(Memberships::UserId)
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    Ok(())
}

macro_rules! impl_iden {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        impl Iden for $name {
            fn unquoted(&self, s: &mut dyn std::fmt::Write) {
                let value = match self {
                    $(Self::$variant => $value,)+
                };
                std::fmt::Write::write_str(s, value).expect("writing identifier cannot fail");
            }
        }
    };
}

#[derive(Copy, Clone)]
enum Users {
    Table,
    Id,
    Handle,
    Email,
    EmailVerified,
    Access,
}

impl_iden!(Users {
    Table => "scope_users",
    Id => "id",
    Handle => "handle",
    Email => "email",
    EmailVerified => "email_verified",
    Access => "access",
});

#[derive(Copy, Clone)]
enum Repositories {
    Table,
    Id,
    OwnerHandle,
    Name,
    OwnerUserId,
    PublicationState,
    DefaultVisibility,
    Settings,
    FirstPushToken,
    GitPushToken,
    PendingImport,
    Policy,
    Graph,
    GitSnapshot,
    StagedUpdate,
    Invitations,
}

impl_iden!(Repositories {
    Table => "scope_repositories",
    Id => "id",
    OwnerHandle => "owner_handle",
    Name => "name",
    OwnerUserId => "owner_user_id",
    PublicationState => "publication_state",
    DefaultVisibility => "default_visibility",
    Settings => "settings",
    FirstPushToken => "first_push_token",
    GitPushToken => "git_push_token",
    PendingImport => "pending_import",
    Policy => "policy",
    Graph => "graph",
    GitSnapshot => "git_snapshot",
    StagedUpdate => "staged_update",
    Invitations => "invitations",
});

#[derive(Copy, Clone)]
enum Memberships {
    Table,
    RepoId,
    UserId,
    Role,
}

impl_iden!(Memberships {
    Table => "scope_repo_memberships",
    RepoId => "repo_id",
    UserId => "user_id",
    Role => "role",
});

#[derive(Copy, Clone)]
enum MetadataLocks {
    Table,
    Key,
    PendingRepoStorageDeletions,
    PendingSourceBlobDeletions,
}

impl_iden!(MetadataLocks {
    Table => "scope_metadata_locks",
    Key => "key",
    PendingRepoStorageDeletions => "pending_repo_storage_deletions",
    PendingSourceBlobDeletions => "pending_source_blob_deletions",
});
