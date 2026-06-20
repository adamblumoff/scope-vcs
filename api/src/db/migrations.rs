use sea_orm_migration::prelude::*;

pub(crate) struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Migration)]
    }
}

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260620_000001_initial_metadata_graph"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(text(Users::Id).primary_key())
                    .col(text(Users::Handle).unique_key())
                    .col(text(Users::Email))
                    .col(boolean(Users::EmailVerified))
                    .col(text(Users::Access))
                    .col(timestamp_with_time_zone(Users::CreatedAt))
                    .col(timestamp_with_time_zone(Users::UpdatedAt))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Repositories::Table)
                    .if_not_exists()
                    .col(text(Repositories::Id).primary_key())
                    .col(text(Repositories::OwnerHandle))
                    .col(text(Repositories::Name))
                    .col(text(Repositories::OwnerUserId))
                    .col(text(Repositories::PublicationState))
                    .col(text(Repositories::DefaultVisibility))
                    .col(text(Repositories::DefaultBranch))
                    .col(timestamp_with_time_zone(Repositories::CreatedAt))
                    .col(timestamp_with_time_zone(Repositories::UpdatedAt))
                    .foreign_key(&mut user_fk(Repositories::Table, Repositories::OwnerUserId))
                    .index(
                        Index::create()
                            .unique()
                            .name("idx_repositories_owner_name")
                            .col(Repositories::OwnerHandle)
                            .col(Repositories::Name),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(RepoMemberships::Table)
                    .if_not_exists()
                    .col(text(RepoMemberships::RepoId))
                    .col(text(RepoMemberships::UserId))
                    .col(text(RepoMemberships::Role))
                    .col(timestamp_with_time_zone(RepoMemberships::CreatedAt))
                    .primary_key(
                        Index::create()
                            .col(RepoMemberships::RepoId)
                            .col(RepoMemberships::UserId),
                    )
                    .foreign_key(&mut repo_fk(
                        RepoMemberships::Table,
                        RepoMemberships::RepoId,
                    ))
                    .foreign_key(&mut user_fk(
                        RepoMemberships::Table,
                        RepoMemberships::UserId,
                    ))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(FirstPushTokens::Table)
                    .if_not_exists()
                    .col(text(FirstPushTokens::RepoId).primary_key())
                    .col(text(FirstPushTokens::TokenHash).unique_key())
                    .col(text(FirstPushTokens::OwnerUserId))
                    .col(timestamp_with_time_zone(FirstPushTokens::CreatedAt))
                    .col(timestamp_with_time_zone(FirstPushTokens::ExpiresAt))
                    .col(timestamp_with_time_zone(FirstPushTokens::UsedAt).null())
                    .foreign_key(&mut repo_fk(
                        FirstPushTokens::Table,
                        FirstPushTokens::RepoId,
                    ))
                    .foreign_key(&mut user_fk(
                        FirstPushTokens::Table,
                        FirstPushTokens::OwnerUserId,
                    ))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(GitPushTokens::Table)
                    .if_not_exists()
                    .col(text(GitPushTokens::RepoId).primary_key())
                    .col(text(GitPushTokens::TokenHash).unique_key())
                    .col(text(GitPushTokens::OwnerUserId))
                    .col(timestamp_with_time_zone(GitPushTokens::CreatedAt))
                    .col(timestamp_with_time_zone(GitPushTokens::RevokedAt).null())
                    .foreign_key(&mut repo_fk(GitPushTokens::Table, GitPushTokens::RepoId))
                    .foreign_key(&mut user_fk(
                        GitPushTokens::Table,
                        GitPushTokens::OwnerUserId,
                    ))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(RepoSettings::Table)
                    .if_not_exists()
                    .col(text(RepoSettings::RepoId).primary_key())
                    .col(boolean(RepoSettings::IncludeIgnoredFiles))
                    .col(boolean(RepoSettings::ReviewPushesBeforeApplying))
                    .foreign_key(&mut repo_fk(RepoSettings::Table, RepoSettings::RepoId))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(VisibilityRules::Table)
                    .if_not_exists()
                    .col(
                        big_integer(VisibilityRules::Id)
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(text(VisibilityRules::RepoId))
                    .col(text(VisibilityRules::Path))
                    .col(text(VisibilityRules::Visibility))
                    .col(json_binary(VisibilityRules::AllowedPrincipalIds))
                    .foreign_key(&mut repo_fk(
                        VisibilityRules::Table,
                        VisibilityRules::RepoId,
                    ))
                    .index(
                        Index::create()
                            .unique()
                            .name("idx_visibility_rules_repo_path")
                            .col(VisibilityRules::RepoId)
                            .col(VisibilityRules::Path),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(LogicalCommits::Table)
                    .if_not_exists()
                    .col(text(LogicalCommits::RepoId))
                    .col(text(LogicalCommits::Id))
                    .col(integer(LogicalCommits::Sequence))
                    .col(json_binary(LogicalCommits::ParentIds))
                    .col(text(LogicalCommits::AuthorId))
                    .col(text(LogicalCommits::AuthorVisibility))
                    .col(text(LogicalCommits::Message))
                    .col(text(LogicalCommits::MixedPolicy))
                    .col(timestamp_with_time_zone(LogicalCommits::CreatedAt))
                    .primary_key(
                        Index::create()
                            .col(LogicalCommits::RepoId)
                            .col(LogicalCommits::Id),
                    )
                    .foreign_key(&mut repo_fk(LogicalCommits::Table, LogicalCommits::RepoId))
                    .foreign_key(&mut user_fk(
                        LogicalCommits::Table,
                        LogicalCommits::AuthorId,
                    ))
                    .index(
                        Index::create()
                            .unique()
                            .name("idx_logical_commits_repo_sequence")
                            .col(LogicalCommits::RepoId)
                            .col(LogicalCommits::Sequence),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(FileChanges::Table)
                    .if_not_exists()
                    .col(big_integer(FileChanges::Id).auto_increment().primary_key())
                    .col(text(FileChanges::RepoId))
                    .col(text(FileChanges::LogicalCommitId))
                    .col(integer(FileChanges::ChangeIndex))
                    .col(text(FileChanges::Path))
                    .col(text(FileChanges::OldBlobObjectKey).null())
                    .col(text(FileChanges::NewBlobObjectKey).null())
                    .col(text(FileChanges::OldContentSha256).null())
                    .col(text(FileChanges::NewContentSha256).null())
                    .foreign_key(&mut logical_commit_fk(
                        FileChanges::Table,
                        (FileChanges::RepoId, FileChanges::LogicalCommitId),
                    ))
                    .index(
                        Index::create()
                            .unique()
                            .name("idx_file_changes_commit_order")
                            .col(FileChanges::RepoId)
                            .col(FileChanges::LogicalCommitId)
                            .col(FileChanges::ChangeIndex),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(PendingImports::Table)
                    .if_not_exists()
                    .col(text(PendingImports::RepoId).primary_key())
                    .col(text(PendingImports::DefaultBranch))
                    .col(text(PendingImports::HeadOid))
                    .col(text(PendingImports::TreeOid))
                    .col(timestamp_with_time_zone(PendingImports::ImportedAt))
                    .foreign_key(&mut repo_fk(PendingImports::Table, PendingImports::RepoId))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(PendingImportFiles::Table)
                    .if_not_exists()
                    .col(
                        big_integer(PendingImportFiles::Id)
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(text(PendingImportFiles::RepoId))
                    .col(integer(PendingImportFiles::FileIndex))
                    .col(text(PendingImportFiles::Path))
                    .col(text(PendingImportFiles::Mode))
                    .col(text(PendingImportFiles::Oid))
                    .col(text(PendingImportFiles::BlobObjectKey).null())
                    .col(text(PendingImportFiles::ContentSha256).null())
                    .col(big_integer(PendingImportFiles::SizeBytes).null())
                    .foreign_key(&mut pending_import_fk(
                        PendingImportFiles::Table,
                        PendingImportFiles::RepoId,
                    ))
                    .index(
                        Index::create()
                            .unique()
                            .name("idx_pending_import_files_repo_order")
                            .col(PendingImportFiles::RepoId)
                            .col(PendingImportFiles::FileIndex),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(StagedUpdates::Table)
                    .if_not_exists()
                    .col(text(StagedUpdates::Id).primary_key())
                    .col(text(StagedUpdates::RepoId).unique_key())
                    .col(text(StagedUpdates::Branch))
                    .col(text(StagedUpdates::BaseLiveCommitId).null())
                    .col(text(StagedUpdates::AuthorId))
                    .col(text(StagedUpdates::Message))
                    .col(timestamp_with_time_zone(StagedUpdates::CreatedAt))
                    .foreign_key(&mut repo_fk(StagedUpdates::Table, StagedUpdates::RepoId))
                    .foreign_key(&mut user_fk(StagedUpdates::Table, StagedUpdates::AuthorId))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(StagedUpdateFiles::Table)
                    .if_not_exists()
                    .col(
                        big_integer(StagedUpdateFiles::Id)
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(text(StagedUpdateFiles::StagedUpdateId))
                    .col(integer(StagedUpdateFiles::FileIndex))
                    .col(text(StagedUpdateFiles::Path))
                    .col(text(StagedUpdateFiles::OldBlobObjectKey).null())
                    .col(text(StagedUpdateFiles::NewBlobObjectKey).null())
                    .col(text(StagedUpdateFiles::Visibility))
                    .col(text(StagedUpdateFiles::ChangeKind))
                    .foreign_key(&mut staged_update_fk(
                        StagedUpdateFiles::Table,
                        StagedUpdateFiles::StagedUpdateId,
                    ))
                    .index(
                        Index::create()
                            .unique()
                            .name("idx_staged_update_files_update_order")
                            .col(StagedUpdateFiles::StagedUpdateId)
                            .col(StagedUpdateFiles::FileIndex),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "staged_update_files",
            "staged_updates",
            "pending_import_files",
            "pending_imports",
            "file_changes",
            "logical_commits",
            "visibility_rules",
            "repo_settings",
            "git_push_tokens",
            "first_push_tokens",
            "repo_memberships",
            "repositories",
            "users",
        ] {
            manager
                .drop_table(
                    Table::drop()
                        .table(Alias::new(table))
                        .if_exists()
                        .to_owned(),
                )
                .await?;
        }

        Ok(())
    }
}

pub(crate) struct Migration;

fn text<T>(column: T) -> ColumnDef
where
    T: IntoIden,
{
    ColumnDef::new(column).text().not_null().to_owned()
}

fn boolean<T>(column: T) -> ColumnDef
where
    T: IntoIden,
{
    ColumnDef::new(column).boolean().not_null().to_owned()
}

fn integer<T>(column: T) -> ColumnDef
where
    T: IntoIden,
{
    ColumnDef::new(column).integer().not_null().to_owned()
}

fn big_integer<T>(column: T) -> ColumnDef
where
    T: IntoIden,
{
    ColumnDef::new(column).big_integer().not_null().to_owned()
}

fn json_binary<T>(column: T) -> ColumnDef
where
    T: IntoIden,
{
    ColumnDef::new(column).json_binary().not_null().to_owned()
}

fn timestamp_with_time_zone<T>(column: T) -> ColumnDef
where
    T: IntoIden,
{
    ColumnDef::new(column)
        .timestamp_with_time_zone()
        .not_null()
        .to_owned()
}

fn repo_fk<T, U>(table: T, column: U) -> ForeignKeyCreateStatement
where
    T: IntoIden + 'static,
    U: IntoIden + 'static,
{
    ForeignKey::create()
        .from(table, column)
        .to(Repositories::Table, Repositories::Id)
        .on_delete(ForeignKeyAction::Cascade)
        .to_owned()
}

fn user_fk<T, U>(table: T, column: U) -> ForeignKeyCreateStatement
where
    T: IntoIden + 'static,
    U: IntoIden + 'static,
{
    ForeignKey::create()
        .from(table, column)
        .to(Users::Table, Users::Id)
        .on_delete(ForeignKeyAction::Restrict)
        .to_owned()
}

fn logical_commit_fk<T, U>(table: T, columns: U) -> ForeignKeyCreateStatement
where
    T: IntoTableRef + 'static,
    U: IdenList + 'static,
{
    ForeignKey::create()
        .from(table, columns)
        .to(
            LogicalCommits::Table,
            (LogicalCommits::RepoId, LogicalCommits::Id),
        )
        .on_delete(ForeignKeyAction::Cascade)
        .to_owned()
}

fn pending_import_fk<T, U>(table: T, column: U) -> ForeignKeyCreateStatement
where
    T: IntoIden + 'static,
    U: IntoIden + 'static,
{
    ForeignKey::create()
        .from(table, column)
        .to(PendingImports::Table, PendingImports::RepoId)
        .on_delete(ForeignKeyAction::Cascade)
        .to_owned()
}

fn staged_update_fk<T, U>(table: T, column: U) -> ForeignKeyCreateStatement
where
    T: IntoIden + 'static,
    U: IntoIden + 'static,
{
    ForeignKey::create()
        .from(table, column)
        .to(StagedUpdates::Table, StagedUpdates::Id)
        .on_delete(ForeignKeyAction::Cascade)
        .to_owned()
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Handle,
    Email,
    EmailVerified,
    Access,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Repositories {
    Table,
    Id,
    OwnerHandle,
    Name,
    OwnerUserId,
    PublicationState,
    DefaultVisibility,
    DefaultBranch,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum RepoMemberships {
    Table,
    RepoId,
    UserId,
    Role,
    CreatedAt,
}

#[derive(DeriveIden)]
enum FirstPushTokens {
    Table,
    RepoId,
    TokenHash,
    OwnerUserId,
    CreatedAt,
    ExpiresAt,
    UsedAt,
}

#[derive(DeriveIden)]
enum GitPushTokens {
    Table,
    RepoId,
    TokenHash,
    OwnerUserId,
    CreatedAt,
    RevokedAt,
}

#[derive(DeriveIden)]
enum RepoSettings {
    Table,
    RepoId,
    IncludeIgnoredFiles,
    ReviewPushesBeforeApplying,
}

#[derive(DeriveIden)]
enum VisibilityRules {
    Table,
    Id,
    RepoId,
    Path,
    Visibility,
    AllowedPrincipalIds,
}

#[derive(DeriveIden)]
enum LogicalCommits {
    Table,
    RepoId,
    Id,
    Sequence,
    ParentIds,
    AuthorId,
    AuthorVisibility,
    Message,
    MixedPolicy,
    CreatedAt,
}

#[derive(DeriveIden)]
enum FileChanges {
    Table,
    Id,
    RepoId,
    LogicalCommitId,
    ChangeIndex,
    Path,
    OldBlobObjectKey,
    NewBlobObjectKey,
    OldContentSha256,
    NewContentSha256,
}

#[derive(DeriveIden)]
enum PendingImports {
    Table,
    RepoId,
    DefaultBranch,
    HeadOid,
    TreeOid,
    ImportedAt,
}

#[derive(DeriveIden)]
enum PendingImportFiles {
    Table,
    Id,
    RepoId,
    FileIndex,
    Path,
    Mode,
    Oid,
    BlobObjectKey,
    ContentSha256,
    SizeBytes,
}

#[derive(DeriveIden)]
enum StagedUpdates {
    Table,
    Id,
    RepoId,
    Branch,
    BaseLiveCommitId,
    AuthorId,
    Message,
    CreatedAt,
}

#[derive(DeriveIden)]
enum StagedUpdateFiles {
    Table,
    Id,
    StagedUpdateId,
    FileIndex,
    Path,
    OldBlobObjectKey,
    NewBlobObjectKey,
    Visibility,
    ChangeKind,
}
