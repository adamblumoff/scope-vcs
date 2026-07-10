use super::entities;
use crate::error::ApiError;
use sea_orm::{ActiveModelTrait, Set};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

const OPERATOR_RESET_TRIGGER: &str = "operator";
static RESET_EVENT_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MetadataResetEvent {
    pub id: String,
    pub reset_at_unix: u64,
    pub trigger: String,
    pub reason: String,
}

pub fn new_operator_metadata_reset_event(reason: &str) -> MetadataResetEvent {
    new_metadata_reset_event(OPERATOR_RESET_TRIGGER, reason)
}

pub async fn insert_metadata_reset_event(
    db: &sea_orm::DatabaseConnection,
    event: &MetadataResetEvent,
) -> Result<(), sea_orm::DbErr> {
    entities::metadata_reset_event::ActiveModel {
        id: Set(event.id.clone()),
        reset_at_unix: Set(i64::try_from(event.reset_at_unix).map_err(|_| {
            sea_orm::DbErr::Custom(
                "metadata reset time exceeds PostgreSQL bigint range".to_string(),
            )
        })?),
        trigger: Set(event.trigger.clone()),
        reason: Set(event.reason.clone()),
    }
    .insert(db)
    .await?;
    Ok(())
}

pub fn metadata_reset_event_from_model(
    model: entities::metadata_reset_event::Model,
) -> Result<MetadataResetEvent, ApiError> {
    Ok(MetadataResetEvent {
        id: model.id,
        reset_at_unix: u64::try_from(model.reset_at_unix)
            .map_err(|_| ApiError::internal_message("metadata reset time cannot be negative"))?,
        trigger: model.trigger,
        reason: model.reason,
    })
}

fn new_metadata_reset_event(
    trigger: impl Into<String>,
    reason: impl Into<String>,
) -> MetadataResetEvent {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let counter = RESET_EVENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let reset_at_unix = now.as_secs();
    MetadataResetEvent {
        id: format!(
            "reset-{}-{}-{}-{counter}",
            reset_at_unix,
            now.subsec_nanos(),
            std::process::id()
        ),
        reset_at_unix,
        trigger: trigger.into(),
        reason: reason.into(),
    }
}
