use super::{RunStore, StoredRunLog, entities};
use crate::error::PostgresError;
use scope_domain::runs::run::RunLogChunk;
use sea_orm::{
    ActiveValue::NotSet, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    TransactionTrait,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendRunLogResult {
    pub log: StoredRunLog,
    pub repo_id: String,
    pub appended: bool,
}

impl RunStore {
    pub async fn append_attempt_log(
        &self,
        chunk: RunLogChunk,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<AppendRunLogResult, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (run, job, mut attempt, steps) =
            super::run_attempt_persistence::locked_attempt_context(&tx, &chunk.attempt_id).await?;
        attempt
            .authenticate_access(&job, token_hash, now_unix)
            .map_err(PostgresError::from)?;
        if attempt.first_truncated_step_index.is_some() {
            return Err(PostgresError::resource_exhausted(
                "run attempt log limit reached",
            ));
        }

        if let Some(existing) = entities::run_log::Entity::find()
            .filter(entities::run_log::Column::AttemptId.eq(&chunk.attempt_id))
            .filter(
                entities::run_log::Column::Sequence.eq(i64::try_from(chunk.sequence)
                    .map_err(|_| PostgresError::invalid_input("run log sequence is too large"))?),
            )
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        {
            let position = entities::i64_to_u64(existing.position, "run log position")?;
            let existing_run_id = existing.run_id.clone();
            let existing_chunk = existing.try_into_domain()?;
            if existing_run_id != run.id
                || existing_chunk.attempt_id != chunk.attempt_id
                || existing_chunk.step_index != chunk.step_index
                || existing_chunk.sequence != chunk.sequence
                || existing_chunk.text != chunk.text
            {
                return Err(PostgresError::conflict(
                    "run log sequence is already used by different content",
                ));
            }
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(AppendRunLogResult {
                log: StoredRunLog {
                    position,
                    run_id: existing_run_id,
                    job_key: job.key.as_str().to_string(),
                    chunk: existing_chunk,
                },
                repo_id: run.workflow.repository_id().to_string(),
                appended: false,
            });
        }

        let expected_sequence = entities::run_log::Entity::find()
            .filter(entities::run_log::Column::AttemptId.eq(&chunk.attempt_id))
            .order_by_desc(entities::run_log::Column::Sequence)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .map_or(1, |last| last.sequence.saturating_add(1));
        if i64::try_from(chunk.sequence).ok() != Some(expected_sequence) {
            return Err(PostgresError::conflict(
                "run log sequence must append without gaps",
            ));
        }
        if !attempt
            .accept_log_chunk(&steps, &chunk)
            .map_err(PostgresError::from)?
        {
            super::run_attempt_persistence::save_attempt(&tx, &attempt).await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Err(PostgresError::resource_exhausted(
                "run attempt log limit reached",
            ));
        }

        let mut model = entities::run_log::Model::from_domain(&run.id, &chunk)?.into_active_model();
        model.position = NotSet;
        let inserted = entities::run_log::Entity::insert(model)
            .exec(&tx)
            .await
            .map_err(|error| {
                super::runs::unique_conflict(error, "run log sequence is already in use")
            })?;
        let position = entities::i64_to_u64(inserted.last_insert_id, "run log position")?;
        super::run_attempt_persistence::save_attempt(&tx, &attempt).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(AppendRunLogResult {
            log: StoredRunLog {
                position,
                run_id: run.id,
                job_key: job.key.as_str().to_string(),
                chunk,
            },
            repo_id: run.workflow.repository_id().to_string(),
            appended: true,
        })
    }
}
