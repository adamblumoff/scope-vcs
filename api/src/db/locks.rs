use super::{METADATA_LOCK_KEY, entities};
use crate::error::ApiError;
use sea_orm::{
    ConnectionTrait, EntityTrait, QuerySelect, Set, TryInsertResult,
    sea_query::{LockType, OnConflict},
};

pub(super) async fn ensure_metadata_lock_row(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), sea_orm::DbErr> {
    match entities::metadata_lock::Entity::insert(entities::metadata_lock::ActiveModel {
        key: Set(METADATA_LOCK_KEY.to_string()),
    })
    .on_conflict(
        OnConflict::column(entities::metadata_lock::Column::Key)
            .do_nothing()
            .to_owned(),
    )
    .do_nothing()
    .exec(db)
    .await?
    {
        TryInsertResult::Empty | TryInsertResult::Conflicted | TryInsertResult::Inserted(_) => {}
    }
    Ok(())
}

pub(super) async fn acquire_metadata_read_lock<C>(conn: &C) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    acquire_metadata_lock(conn, LockType::Share).await
}

pub(super) async fn acquire_metadata_write_lock<C>(conn: &C) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    acquire_metadata_lock(conn, LockType::Update).await
}

async fn acquire_metadata_lock<C>(conn: &C, lock: LockType) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let row = entities::metadata_lock::Entity::find_by_id(METADATA_LOCK_KEY.to_string())
        .lock(lock)
        .one(conn)
        .await
        .map_err(ApiError::internal)?;
    if row.is_none() {
        return Err(ApiError::internal_message("metadata lock row is missing"));
    }
    Ok(())
}
