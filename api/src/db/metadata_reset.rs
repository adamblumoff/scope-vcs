use super::{entities, load_catalog_async, schema};
use crate::error::ApiError;
use sea_orm::{ActiveModelTrait, Set};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

const OPERATOR_RESET_TRIGGER: &str = "operator";
static RESET_EVENT_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct MetadataResetEvent {
    pub(crate) id: String,
    pub(crate) reset_at_unix: u64,
    pub(crate) trigger: String,
    pub(crate) reason: String,
}

pub(super) fn new_operator_metadata_reset_event(reason: &str) -> MetadataResetEvent {
    new_metadata_reset_event(OPERATOR_RESET_TRIGGER, reason)
}

pub(super) async fn reset_stale_pre_alpha_metadata(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), sea_orm::DbErr> {
    match load_catalog_async(db).await {
        Ok(_) => Ok(()),
        Err(error) if is_stale_pre_alpha_metadata_error(&error) => {
            eprintln!(
                "resetting stale pre-alpha Scope metadata after incompatible persisted shape: {}",
                error.message
            );
            schema::reset_metadata_schema(db).await?;
            schema::migrate_metadata_schema(db).await?;
            super::ensure_metadata_lock_row(db).await?;
            let event = new_metadata_reset_event(
                "startup_stale_pre_alpha",
                format!("incompatible persisted shape: {}", error.message),
            );
            insert_metadata_reset_event(db, &event).await
        }
        Err(error) => Err(sea_orm::DbErr::Custom(format!(
            "failed to load Scope metadata after migration: {}",
            error.message
        ))),
    }
}

pub(super) async fn insert_metadata_reset_event(
    db: &sea_orm::DatabaseConnection,
    event: &MetadataResetEvent,
) -> Result<(), sea_orm::DbErr> {
    entities::metadata_reset_event::ActiveModel {
        id: Set(event.id.clone()),
        reset_at_unix: Set(event.reset_at_unix as i64),
        trigger: Set(event.trigger.clone()),
        reason: Set(event.reason.clone()),
    }
    .insert(db)
    .await?;
    Ok(())
}

pub(super) fn metadata_reset_event_from_model(
    model: entities::metadata_reset_event::Model,
) -> MetadataResetEvent {
    MetadataResetEvent {
        id: model.id,
        reset_at_unix: model.reset_at_unix.max(0) as u64,
        trigger: model.trigger,
        reason: model.reason,
    }
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

fn is_stale_pre_alpha_metadata_error(error: &ApiError) -> bool {
    matches!(
        error.message.as_str(),
        "missing field `visibility`"
            | "missing field `visibility_changes`"
            | "missing field `visibility_events`"
            | "missing field `line_count`"
            | "missing field `line_diff`"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_pre_alpha_reset_is_limited_to_known_visibility_shape_error() {
        assert!(is_stale_pre_alpha_metadata_error(
            &ApiError::internal_message("missing field `visibility`")
        ));
        assert!(is_stale_pre_alpha_metadata_error(
            &ApiError::internal_message("missing field `visibility_changes`")
        ));
        assert!(is_stale_pre_alpha_metadata_error(
            &ApiError::internal_message("missing field `visibility_events`")
        ));
        assert!(is_stale_pre_alpha_metadata_error(
            &ApiError::internal_message("missing field `line_count`")
        ));
        assert!(is_stale_pre_alpha_metadata_error(
            &ApiError::internal_message("missing field `line_diff`")
        ));
        assert!(!is_stale_pre_alpha_metadata_error(
            &ApiError::internal_message("missing field `owner_user_id`")
        ));
        assert!(!is_stale_pre_alpha_metadata_error(
            &ApiError::internal_message("database connection failed")
        ));
    }
}
