use sea_orm_migration::prelude::Iden;

pub mod auth;
pub mod collaboration;
pub mod jobs;
pub mod read_models;
pub mod repositories;
pub mod requests;

#[derive(Copy, Clone)]
pub struct SchemaIden(&'static str);

impl SchemaIden {
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }
}

impl Iden for SchemaIden {
    fn unquoted(&self, s: &mut dyn std::fmt::Write) {
        std::fmt::Write::write_str(s, self.0).expect("writing identifier cannot fail");
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ColumnType {
    BigInteger,
    Boolean,
    Integer,
    JsonBinary,
    String,
    Text,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ColumnSpec {
    pub name: &'static str,
    pub column_type: ColumnType,
    pub nullable: bool,
    pub unique: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PrimaryKeySpec {
    Inline(&'static str),
    Composite {
        name: &'static str,
        columns: &'static [&'static str],
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct IndexSpec {
    pub name: &'static str,
    pub columns: &'static [&'static str],
    pub unique: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ForeignKeyActionSpec {
    Cascade,
    SetNull,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ForeignKeySpec {
    pub name: &'static str,
    pub column: &'static str,
    pub to_table: &'static str,
    pub to_column: &'static str,
    pub on_delete: ForeignKeyActionSpec,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TableSpec {
    pub name: &'static str,
    pub columns: &'static [ColumnSpec],
    pub primary_key: PrimaryKeySpec,
    pub indexes: &'static [IndexSpec],
    pub foreign_keys: &'static [ForeignKeySpec],
    pub counts_for_catalog_rows: bool,
    pub reset_order: u16,
}

impl TableSpec {
    pub fn column_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.columns.iter().map(|column| column.name)
    }

    pub fn inline_primary_key_column(&self) -> Option<&'static str> {
        match self.primary_key {
            PrimaryKeySpec::Inline(column) => Some(column),
            PrimaryKeySpec::Composite { .. } => None,
        }
    }
}

pub const fn required_column(name: &'static str, column_type: ColumnType) -> ColumnSpec {
    ColumnSpec {
        name,
        column_type,
        nullable: false,
        unique: false,
    }
}

pub const fn nullable_column(name: &'static str, column_type: ColumnType) -> ColumnSpec {
    ColumnSpec {
        name,
        column_type,
        nullable: true,
        unique: false,
    }
}

pub const fn unique_column(name: &'static str, column_type: ColumnType) -> ColumnSpec {
    ColumnSpec {
        name,
        column_type,
        nullable: false,
        unique: true,
    }
}

pub const fn inline_primary_key(column: &'static str) -> PrimaryKeySpec {
    PrimaryKeySpec::Inline(column)
}

pub const fn composite_primary_key(
    name: &'static str,
    columns: &'static [&'static str],
) -> PrimaryKeySpec {
    PrimaryKeySpec::Composite { name, columns }
}

pub const fn index(name: &'static str, columns: &'static [&'static str]) -> IndexSpec {
    IndexSpec {
        name,
        columns,
        unique: false,
    }
}

pub const fn unique_index(name: &'static str, columns: &'static [&'static str]) -> IndexSpec {
    IndexSpec {
        name,
        columns,
        unique: true,
    }
}

pub const fn cascade_fk(
    name: &'static str,
    column: &'static str,
    to_table: &'static str,
    to_column: &'static str,
) -> ForeignKeySpec {
    ForeignKeySpec {
        name,
        column,
        to_table,
        to_column,
        on_delete: ForeignKeyActionSpec::Cascade,
    }
}

pub const fn set_null_fk(
    name: &'static str,
    column: &'static str,
    to_table: &'static str,
    to_column: &'static str,
) -> ForeignKeySpec {
    ForeignKeySpec {
        name,
        column,
        to_table,
        to_column,
        on_delete: ForeignKeyActionSpec::SetNull,
    }
}

const SCHEMA_TABLE_GROUPS: &[&[TableSpec]] = &[
    jobs::LOCK_AND_CLEANUP_TABLES,
    auth::TABLES,
    repositories::TABLES,
    requests::TABLES,
    read_models::TABLES,
    jobs::OUTBOX_TABLES,
    collaboration::TABLES,
];

pub fn metadata_reset_events_table() -> &'static TableSpec {
    &jobs::METADATA_RESET_EVENTS
}

pub fn schema_tables() -> impl Iterator<Item = &'static TableSpec> {
    SCHEMA_TABLE_GROUPS.iter().flat_map(|group| group.iter())
}

pub fn catalog_row_tables() -> impl Iterator<Item = &'static TableSpec> {
    schema_tables().filter(|table| table.counts_for_catalog_rows)
}

pub fn metadata_reset_tables() -> Vec<&'static str> {
    let mut tables = schema_tables()
        .map(|table| (table.reset_order, table.name))
        .collect::<Vec<_>>();
    tables.sort_unstable_by_key(|(reset_order, _)| *reset_order);
    tables.into_iter().map(|(_, name)| name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_contract_inventory_covers_all_reset_tables() {
        let mut reset_tables = metadata_reset_tables();
        reset_tables.sort_unstable();
        reset_tables.dedup();

        for table in schema_tables() {
            assert!(
                reset_tables.contains(&table.name),
                "reset list missing {}",
                table.name
            );
        }

        assert!(
            !reset_tables.contains(&metadata_reset_events_table().name),
            "reset events are append-only across destructive schema resets"
        );
    }

    #[test]
    fn schema_contract_inventory_marks_catalog_row_tables() {
        let catalog_tables = catalog_row_tables()
            .map(|table| table.name)
            .collect::<Vec<_>>();

        assert_eq!(
            catalog_tables,
            vec![
                jobs::REPO_STORAGE_CLEANUP_JOBS.name,
                jobs::SOURCE_BLOB_CLEANUP_JOBS.name,
                auth::USERS.name,
                auth::AUTH_IDENTITIES.name,
                auth::CLI_DEVICE_LOGINS.name,
                auth::CLI_BROWSER_LOGINS.name,
                auth::CLI_EXCHANGE_GRANTS.name,
                auth::CLI_SESSIONS.name,
                repositories::REPOSITORIES.name,
                repositories::REPOSITORY_SETTINGS.name,
                repositories::REPOSITORY_FIRST_PUSH_TOKENS.name,
                repositories::REPOSITORY_GIT_PUSH_TOKENS.name,
                repositories::REPOSITORY_GIT_SNAPSHOTS.name,
                requests::REQUESTS.name,
                requests::REQUEST_EVENTS.name,
                requests::USER_CREDIT_ACCOUNTS.name,
                requests::CREDIT_LEDGER_ENTRIES.name,
                collaboration::REPOSITORY_MEMBERS.name,
                collaboration::REPOSITORY_INVITES.name,
            ]
        );
    }

    #[test]
    fn schema_contract_inventory_has_unique_names() {
        let mut reset_orders = std::collections::BTreeSet::new();
        let mut tables = std::collections::BTreeSet::new();
        for table in schema_tables() {
            assert!(tables.insert(table.name), "duplicate table {}", table.name);
            assert!(
                reset_orders.insert(table.reset_order),
                "duplicate reset order {}",
                table.reset_order
            );
            assert!(!table.columns.is_empty(), "{} has no columns", table.name);

            let mut columns = std::collections::BTreeSet::new();
            for column in table.columns {
                assert!(
                    columns.insert(column.name),
                    "duplicate column {}.{}",
                    table.name,
                    column.name
                );
            }
        }
    }
}
