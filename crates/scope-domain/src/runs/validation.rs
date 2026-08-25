use crate::error::DomainError;

pub(super) fn validate_sha256_hash(label: &str, value: &str) -> Result<(), DomainError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DomainError::invalid_input(format!(
            "{label} must be a SHA-256 hex digest"
        )));
    }
    Ok(())
}

pub(super) fn required(label: &str, value: String) -> Result<String, DomainError> {
    if value.trim().is_empty() {
        Err(DomainError::invalid_input(format!("{label} is required")))
    } else {
        Ok(value)
    }
}
