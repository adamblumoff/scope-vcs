use super::*;

#[derive(Copy, Clone)]
pub enum ProjectionReadModels {
    Table,
    RepoId,
    RepoVersion,
    Source,
    Audience,
    RebuiltAtUnix,
    FileCount,
}

impl_iden!(ProjectionReadModels {
    Table => "scope_projection_read_models",
    RepoId => "repo_id",
    RepoVersion => "repo_version",
    Source => "source",
    Audience => "audience",
    RebuiltAtUnix => "rebuilt_at_unix",
    FileCount => "file_count",
});

#[derive(Copy, Clone)]
pub enum ProjectionFiles {
    Table,
    RepoId,
    RepoVersion,
    Source,
    Audience,
    PathKey,
    Path,
    Oid,
    Visibility,
}

impl_iden!(ProjectionFiles {
    Table => "scope_projection_files",
    RepoId => "repo_id",
    RepoVersion => "repo_version",
    Source => "source",
    Audience => "audience",
    PathKey => "path_key",
    Path => "path",
    Oid => "oid",
    Visibility => "visibility",
});

const PROJECTION_READ_MODEL_COLUMNS: &[&str] = &[
    ProjectionReadModels::RepoId.as_str(),
    ProjectionReadModels::RepoVersion.as_str(),
    ProjectionReadModels::Source.as_str(),
    ProjectionReadModels::Audience.as_str(),
    ProjectionReadModels::RebuiltAtUnix.as_str(),
    ProjectionReadModels::FileCount.as_str(),
];

const PROJECTION_FILE_COLUMNS: &[&str] = &[
    ProjectionFiles::RepoId.as_str(),
    ProjectionFiles::RepoVersion.as_str(),
    ProjectionFiles::Source.as_str(),
    ProjectionFiles::Audience.as_str(),
    ProjectionFiles::PathKey.as_str(),
    ProjectionFiles::Path.as_str(),
    ProjectionFiles::Oid.as_str(),
    ProjectionFiles::Visibility.as_str(),
];

pub const TABLES: &[MetadataTableSpec] = &[
    MetadataTableSpec {
        table: ProjectionReadModels::Table.as_str(),
        columns: PROJECTION_READ_MODEL_COLUMNS,
        counts_for_catalog_rows: false,
    },
    MetadataTableSpec {
        table: ProjectionFiles::Table.as_str(),
        columns: PROJECTION_FILE_COLUMNS,
        counts_for_catalog_rows: false,
    },
];
