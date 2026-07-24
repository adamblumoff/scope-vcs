use crate::error::ApiError;

pub const REQUEST_ACTIVITY_PAGE_MAX_EVENTS: usize = 50;
pub const REQUEST_ASSESSMENT_BODY_MAX_BYTES: usize = 64 * 1024;
pub const REQUEST_DISCUSSION_BODY_MAX_BYTES: usize = 64 * 1024;
pub const REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES: usize = 128;
pub const REQUEST_DISCUSSION_REPLY_MAX_DEPTH: u16 = 16;
pub const REQUEST_DESCRIPTION_MAX_BYTES: usize = 256 * 1024;
pub const REQUEST_LIST_DEFAULT_PAGE_SIZE: usize = 50;
pub const REQUEST_LIST_MAX_PAGE_SIZE: usize = 100;
pub const REQUEST_TIMELINE_BODY_MAX_BYTES: usize = 16 * 1024;
pub const REQUEST_TITLE_MAX_BYTES: usize = 256;

pub(crate) fn validate_required_body(label: &str, value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::bad_request(format!("{label} is required")));
    }
    Ok(())
}

pub(crate) fn validate_body_size(
    label: &str,
    value: &str,
    max_bytes: usize,
) -> Result<(), ApiError> {
    if value.len() > max_bytes {
        return Err(ApiError::bad_request(format!(
            "{label} exceeds {max_bytes} bytes"
        )));
    }
    Ok(())
}
