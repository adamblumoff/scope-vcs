use scope_domain::requests::{RequestEventPayload, request_identity_audit_fact};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};
use serde::Deserialize;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0011_compact_request_started_events"
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyStartedEnvelope {
    #[serde(rename = "Started")]
    started: LegacyStarted,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyStarted {
    title: String,
    description_markdown: String,
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("LOCK TABLE scope_request_events IN ACCESS EXCLUSIVE MODE")
            .await?;
        let mut after_id: Option<String> = None;
        loop {
            let events = db
                .query_all(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "
                        SELECT id, payload
                        FROM scope_request_events
                        WHERE kind = 'Started'
                          AND ($1::text IS NULL OR id > $1)
                        ORDER BY id
                        LIMIT 100
                    ",
                    [after_id.clone().into()],
                ))
                .await?;
            if events.is_empty() {
                break;
            }
            for event in events {
                let id = event.try_get::<String>("", "id")?;
                let payload = event.try_get::<serde_json::Value>("", "payload")?;
                let legacy =
                    serde_json::from_value::<LegacyStartedEnvelope>(payload).map_err(|error| {
                        DbErr::Migration(format!(
                            "request Started event {id} has an invalid legacy payload: {error}"
                        ))
                    })?;
                let identity = request_identity_audit_fact(
                    &legacy.started.title,
                    &legacy.started.description_markdown,
                )
                .map_err(|error| DbErr::Migration(error.to_string()))?;
                let payload = serde_json::to_value(RequestEventPayload::Started { identity })
                    .map_err(|error| DbErr::Migration(error.to_string()))?;
                db.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "UPDATE scope_request_events SET payload = $1 WHERE id = $2",
                    [payload.into(), id.clone().into()],
                ))
                .await?;
                after_id = Some(id);
            }
        }
        Ok(())
    }
}
