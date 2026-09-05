use super::{RunStore, entities};
use crate::error::PostgresError;
use scope_domain::runs::{log::RunLogChunk, workflow::definition::WorkflowJobId};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, Statement,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRunLog {
    pub position: u64,
    pub run_id: String,
    pub job_key: String,
    pub chunk: RunLogChunk,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentRunLogs {
    pub logs: Vec<StoredRunLog>,
    pub truncated_in_view: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredAttemptStepLogs {
    pub logs: Vec<StoredRunLog>,
    pub logs_truncated: bool,
    pub has_earlier: bool,
    pub has_more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepLogCursor {
    Tail,
    Before(u64),
    After(u64),
}

pub const STEP_LOG_PAGE_BYTES: u64 = 512 * 1024;

#[derive(Clone, Debug, FromQueryResult)]
struct RunLogReadRow {
    position: i64,
    run_id: String,
    attempt_id: String,
    job_key: String,
    step_index: i32,
    sequence: i64,
    text: String,
    created_at_unix: i64,
}

impl RunLogReadRow {
    fn try_into_stored(self) -> Result<StoredRunLog, PostgresError> {
        let job_key = WorkflowJobId::parse(self.job_key).map_err(PostgresError::invalid_input)?;
        Ok(StoredRunLog {
            position: entities::i64_to_u64(self.position, "run log position")?,
            run_id: self.run_id,
            job_key: job_key.as_str().to_string(),
            chunk: RunLogChunk::new(
                self.attempt_id,
                entities::i32_to_u32(self.step_index, "run log step index")?,
                entities::i64_to_u64(self.sequence, "run log sequence")?,
                self.text,
                entities::i64_to_u64(self.created_at_unix, "run log creation time")?,
            )
            .map_err(PostgresError::invalid_input)?,
        })
    }
}

impl RunStore {
    pub async fn attempt_step_logs(
        &self,
        run_id: &str,
        attempt_id: &str,
        step_index: u32,
        cursor: StepLogCursor,
        limit: u64,
    ) -> Result<StoredAttemptStepLogs, PostgresError> {
        let tx = super::begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let attempt = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
            .filter(entities::run_attempt::Column::RunId.eq(run_id))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("run attempt not found"))?;
        let job_key = WorkflowJobId::parse(attempt.job_key)
            .map_err(PostgresError::invalid_input)?
            .as_str()
            .to_string();
        let step_index = i32::try_from(step_index)
            .map_err(|_| PostgresError::invalid_input("step index is too large"))?;
        let step_exists =
            entities::run_attempt_step::Entity::find_by_id((attempt_id.to_string(), step_index))
                .one(&tx)
                .await
                .map_err(PostgresError::internal)?
                .is_some();
        if !step_exists {
            return Err(PostgresError::not_found("run attempt step not found"));
        }
        let (position, comparison, order) = match cursor {
            StepLogCursor::Tail => (0, ">", "DESC"),
            StepLogCursor::Before(position) => (position, "<", "DESC"),
            StepLogCursor::After(position) => (position, ">", "ASC"),
        };
        let position = i64::try_from(position)
            .map_err(|_| PostgresError::invalid_input("run log cursor is too large"))?;
        let limit = i64::try_from(limit.clamp(1, 128))
            .map_err(|_| PostgresError::invalid_input("run log limit is too large"))?;
        // Bound row work before computing the byte window. Only the retained chunks cross
        // the database connection, including for a tail read of a long completed step.
        let logs = entities::run_log::Model::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "WITH candidates AS (
                SELECT position, SUM(octet_length(text)) OVER (ORDER BY position {order}) AS bytes
                FROM (SELECT position, text FROM scope_run_logs
                    WHERE run_id = $1 AND attempt_id = $2 AND step_index = $3
                      AND position {comparison} $4
                    ORDER BY position {order} LIMIT $5) bounded
            ) SELECT log.* FROM scope_run_logs log
                JOIN candidates page ON page.position = log.position
                WHERE page.bytes <= $6 ORDER BY log.position ASC"
            ),
            [
                run_id.into(),
                attempt_id.into(),
                step_index.into(),
                position.into(),
                limit.into(),
                (STEP_LOG_PAGE_BYTES as i64).into(),
            ],
        ))
        .all(&tx)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|model| -> Result<StoredRunLog, PostgresError> {
            let position = entities::i64_to_u64(model.position, "run log position")?;
            let run_id = model.run_id.clone();
            Ok(StoredRunLog {
                position,
                run_id,
                job_key: job_key.clone(),
                chunk: model.try_into_domain()?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
        let first = logs.first().map_or(position, |log| log.position as i64);
        let last = logs.last().map_or(position, |log| log.position as i64);
        let bounds = tx.query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT EXISTS(SELECT 1 FROM scope_run_logs WHERE attempt_id = $1 AND step_index = $2 AND position < $3) AS has_earlier,
                    EXISTS(SELECT 1 FROM scope_run_logs WHERE attempt_id = $1 AND step_index = $2 AND position > $4) AS has_more",
            [attempt_id.into(), step_index.into(), first.into(), last.into()],
        )).await.map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("run log page bounds missing"))?;
        let has_earlier = bounds
            .try_get("", "has_earlier")
            .map_err(PostgresError::internal)?;
        let has_more = bounds
            .try_get("", "has_more")
            .map_err(PostgresError::internal)?;
        let logs_truncated = attempt
            .first_truncated_step_index
            .is_some_and(|first| step_index >= first);
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(StoredAttemptStepLogs {
            logs,
            logs_truncated,
            has_earlier,
            has_more,
        })
    }

    pub async fn run_logs_after(
        &self,
        run_id: &str,
        after: u64,
        limit: u64,
    ) -> Result<Vec<StoredRunLog>, PostgresError> {
        let after = i64::try_from(after)
            .map_err(|_| PostgresError::invalid_input("run log cursor is too large"))?;
        let limit = i64::try_from(limit)
            .map_err(|_| PostgresError::invalid_input("run log page limit is too large"))?;
        joined_run_logs(
            self.db.as_ref(),
            "WHERE log.run_id = $1 AND log.position > $2
             ORDER BY log.position ASC
             LIMIT $3",
            [run_id.into(), after.into(), limit.into()],
        )
        .await
    }

    pub async fn recent_run_logs(
        &self,
        run_id: &str,
        limit: u64,
    ) -> Result<RecentRunLogs, PostgresError> {
        let fetch_limit = limit.saturating_add(1);
        let fetch_limit = i64::try_from(fetch_limit)
            .map_err(|_| PostgresError::invalid_input("recent run log limit is too large"))?;
        let mut logs = joined_run_logs(
            self.db.as_ref(),
            "WHERE log.run_id = $1
             ORDER BY log.position DESC
             LIMIT $2",
            [run_id.into(), fetch_limit.into()],
        )
        .await?;
        let truncated_in_view = logs.len() as u64 > limit;
        if truncated_in_view {
            logs.pop();
        }
        logs.reverse();
        Ok(RecentRunLogs {
            logs,
            truncated_in_view,
        })
    }

    pub async fn next_attempt_log_sequence(&self, attempt_id: &str) -> Result<u64, PostgresError> {
        let last = entities::run_log::Entity::find()
            .filter(entities::run_log::Column::AttemptId.eq(attempt_id))
            .order_by_desc(entities::run_log::Column::Sequence)
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        match last {
            Some(log) => entities::i64_to_u64(log.sequence, "run log sequence")?
                .checked_add(1)
                .ok_or_else(|| PostgresError::conflict("run log sequence overflow")),
            None => Ok(1),
        }
    }
}

async fn joined_run_logs<C, const N: usize>(
    conn: &C,
    clause: &str,
    values: [sea_orm::Value; N],
) -> Result<Vec<StoredRunLog>, PostgresError>
where
    C: ConnectionTrait,
{
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "SELECT log.position,
                    log.run_id,
                    log.attempt_id,
                    attempt.job_key,
                    log.step_index,
                    log.sequence,
                    log.text,
                    log.created_at_unix
               FROM scope_run_logs AS log
               JOIN scope_run_attempts AS attempt ON attempt.id = log.attempt_id
               {clause}"
        ),
        values,
    );
    RunLogReadRow::find_by_statement(statement)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(RunLogReadRow::try_into_stored)
        .collect()
}
