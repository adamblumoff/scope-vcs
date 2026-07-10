use super::{METADATA_LOCK_KEY, entities};
use crate::error::ApiError;
use sea_orm::{
    ConnectionTrait, EntityTrait, QuerySelect, Set, TryInsertResult,
    sea_query::{LockType, OnConflict},
};

pub async fn ensure_metadata_lock_row(
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

pub async fn acquire_aggregate_lock<C>(conn: &C, namespace: &str, id: &str) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let key = format!("{namespace}:{id}");
    entities::metadata_lock::Entity::insert(entities::metadata_lock::ActiveModel {
        key: Set(key.clone()),
    })
    .on_conflict(
        OnConflict::column(entities::metadata_lock::Column::Key)
            .do_nothing()
            .to_owned(),
    )
    .do_nothing()
    .exec(conn)
    .await
    .map_err(ApiError::internal)?;
    let row = entities::metadata_lock::Entity::find_by_id(key)
        .lock(LockType::Update)
        .one(conn)
        .await
        .map_err(ApiError::internal)?;
    if row.is_none() {
        return Err(ApiError::internal_message(
            "aggregate lock row disappeared during acquisition",
        ));
    }
    Ok(())
}
