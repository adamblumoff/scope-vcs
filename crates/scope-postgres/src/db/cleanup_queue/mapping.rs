use crate::error::PostgresError;
use scope_domain::content_ref::ContentRef;

pub(super) fn encode_content_ref(content_ref: &ContentRef) -> Result<String, PostgresError> {
    serde_json::to_string(content_ref).map_err(PostgresError::internal)
}

pub(super) fn u64_to_i64(value: u64) -> Result<i64, PostgresError> {
    i64::try_from(value).map_err(|_| PostgresError::internal_message("timestamp exceeds i64 range"))
}
