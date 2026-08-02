use super::{
    RequestStore, entities,
    request_access::{ensure_user_exists, lock_request_repository},
};
use crate::error::PostgresError;
use scope_domain::requests::{
    CreateRequestRatingInput, RequestRating, RequestReputation, create_request_rating,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, Statement, TransactionTrait,
};

impl RequestStore {
    pub async fn request_ratings(
        &self,
        request_id: &str,
    ) -> Result<Vec<RequestRating>, PostgresError> {
        ratings_for_request(self.db.as_ref(), request_id).await
    }

    pub async fn create_request_rating(
        &self,
        input: CreateRequestRatingInput,
    ) -> Result<RequestRating, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (_repo, request) = lock_request_repository(&tx, &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let ratings = ratings_for_request(&tx, &request.id).await?;
        let rating = create_request_rating(&request, &ratings, input)?;
        entities::request_rating::Model::from_domain(&rating)?
            .into_active_model()
            .insert(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(rating)
    }

    pub async fn request_reputation(
        &self,
        user_id: &str,
    ) -> Result<RequestReputation, PostgresError> {
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                    SELECT COALESCE(SUM(score), 0)::bigint AS score_sum,
                           COUNT(*)::bigint AS rating_count
                    FROM scope_request_ratings
                    WHERE subject_user_id = $1
                "#,
                [user_id.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("request reputation row is missing"))?;
        let score_sum = row
            .try_get::<i64>("", "score_sum")
            .map_err(PostgresError::internal)?
            .try_into()
            .map_err(PostgresError::internal)?;
        let rating_count = row
            .try_get::<i64>("", "rating_count")
            .map_err(PostgresError::internal)?
            .try_into()
            .map_err(PostgresError::internal)?;
        RequestReputation::from_totals(score_sum, rating_count).map_err(Into::into)
    }
}

async fn ratings_for_request<C>(
    conn: &C,
    request_id: &str,
) -> Result<Vec<RequestRating>, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    entities::request_rating::Entity::find()
        .filter(entities::request_rating::Column::RequestId.eq(request_id))
        .order_by_asc(entities::request_rating::Column::CreatedAtUnix)
        .order_by_asc(entities::request_rating::Column::Id)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::request_rating::Model::try_into_domain)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{
        generated_ids::test_generated_id,
        requests::tests::{postgres_store, start_public_request},
    };
    use scope_domain::requests::{CloseRequestInput, SubmitRequestInput};

    #[tokio::test]
    async fn terminal_participants_persist_at_most_one_immutable_rating_each() {
        let store = postgres_store();
        start_public_request(&store).await;
        store
            .requests()
            .submit_request(SubmitRequestInput {
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                actor_is_author: false,
                actor_can_submit: false,
                event_id: "event_submitted".to_string(),
                now_unix: 4,
            })
            .await
            .unwrap();
        store
            .requests()
            .close_request(
                CloseRequestInput {
                    request_id: "req_1".to_string(),
                    actor_user_id: "user_owner".to_string(),
                    actor_is_author: false,
                    actor_is_maintainer: false,
                    event_id: "event_closed".to_string(),
                    now_unix: 5,
                },
                &test_generated_id,
            )
            .await
            .unwrap();

        let author_rating = store
            .requests()
            .create_request_rating(CreateRequestRatingInput {
                id: "rating_author".to_string(),
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                score: 5,
                reason: "  Thoughtful review  ".to_string(),
                now_unix: 6,
            })
            .await
            .unwrap();
        assert_eq!(author_rating.subject_user_id, "user_owner");
        assert_eq!(author_rating.reason, "Thoughtful review");
        assert!(
            store
                .requests()
                .create_request_rating(CreateRequestRatingInput {
                    id: "rating_author_again".to_string(),
                    request_id: "req_1".to_string(),
                    actor_user_id: "user_public".to_string(),
                    score: 4,
                    reason: "Again".to_string(),
                    now_unix: 7,
                })
                .await
                .is_err()
        );
        let maintainer_rating = store
            .requests()
            .create_request_rating(CreateRequestRatingInput {
                id: "rating_maintainer".to_string(),
                request_id: "req_1".to_string(),
                actor_user_id: "user_owner".to_string(),
                score: 4,
                reason: "Useful contribution".to_string(),
                now_unix: 8,
            })
            .await
            .unwrap();
        assert_eq!(maintainer_rating.subject_user_id, "user_public");
        assert_eq!(
            store
                .requests()
                .request_ratings("req_1")
                .await
                .unwrap()
                .len(),
            2
        );

        let request_store = store.requests();
        let mut second_request = request_store.request_by_id("req_1").await.unwrap().unwrap();
        second_request.id = "req_2".to_string();
        second_request.name = "second-request".to_string();
        entities::request::Model::from_domain(&second_request)
            .unwrap()
            .into_active_model()
            .insert(request_store.db.as_ref())
            .await
            .unwrap();
        request_store
            .create_request_rating(CreateRequestRatingInput {
                id: "rating_author_second_request".to_string(),
                request_id: "req_2".to_string(),
                actor_user_id: "user_public".to_string(),
                score: 3,
                reason: "Another clear review".to_string(),
                now_unix: 9,
            })
            .await
            .unwrap();
        assert_eq!(
            store
                .requests()
                .request_reputation("user_owner")
                .await
                .unwrap(),
            RequestReputation {
                score_sum: 8,
                rating_count: 2,
            }
        );
        assert_eq!(
            store
                .requests()
                .request_reputation("user_public")
                .await
                .unwrap(),
            RequestReputation {
                score_sum: 4,
                rating_count: 1,
            }
        );
        assert_eq!(
            store
                .requests()
                .request_reputation("user_member")
                .await
                .unwrap(),
            RequestReputation::default()
        );
    }
}
