use super::{METADATA_LOCK_KEY, entities};
use crate::error::ApiError;
use sea_orm::{
    ConnectionTrait, EntityTrait, QuerySelect, Set, TryInsertResult,
    sea_query::{LockType, OnConflict},
};
#[cfg(test)]
use sea_orm::{DatabaseBackend, Statement};

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
    #[cfg(test)]
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT set_config('application_name', $1, true)",
        [format!("scope-test-lock:{namespace}").into()],
    ))
    .await
    .map_err(ApiError::internal)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{MetadataStore, TestDatabaseTarget};
    use sea_orm::TransactionTrait;
    use std::time::Duration;

    #[tokio::test]
    async fn aggregate_locks_serialize_same_key_without_blocking_other_keys() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        let held = store.db.begin().await.unwrap();
        acquire_aggregate_lock(&held, "repository", "owner/one")
            .await
            .unwrap();

        let same_store = store.clone();
        let same = tokio::spawn(async move {
            let tx = same_store.db.begin().await.unwrap();
            acquire_aggregate_lock(&tx, "repository", "owner/one")
                .await
                .unwrap();
            tx.commit().await.unwrap();
        });
        let other_store = store.clone();
        let other = tokio::spawn(async move {
            let tx = other_store.db.begin().await.unwrap();
            acquire_aggregate_lock(&tx, "repository", "owner/two")
                .await
                .unwrap();
            tx.commit().await.unwrap();
        });

        tokio::time::timeout(Duration::from_secs(2), other)
            .await
            .expect("different aggregate key should not block")
            .unwrap();
        assert!(!same.is_finished());
        held.commit().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), same)
            .await
            .expect("same aggregate key should proceed after release")
            .unwrap();
    }
}
