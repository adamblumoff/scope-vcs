use scope_postgres::db::GeneratedIdKind;

pub(crate) fn generate_persistence_id(kind: GeneratedIdKind) -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    let random = hex::encode(bytes);
    Ok(match kind {
        GeneratedIdKind::CleanupGeneration => random,
        GeneratedIdKind::OutboxJob => format!("outbox_{random}"),
        GeneratedIdKind::RepositoryIncarnation => format!("repoi_{random}"),
    })
}

pub(crate) fn generate_prefixed_id(prefix: &str) -> Result<String, crate::error::ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        crate::error::ApiError::internal_message(format!("failed to generate identifier: {error}"))
    })?;
    Ok(format!("{prefix}{}", hex::encode(bytes)))
}
