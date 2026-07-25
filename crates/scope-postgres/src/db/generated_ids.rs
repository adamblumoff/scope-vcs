use crate::error::PostgresError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneratedIdKind {
    CleanupGeneration,
    OutboxJob,
}

pub trait GeneratedIdSource: Send + Sync {
    fn generate(&self, kind: GeneratedIdKind) -> Result<String, String>;
}

impl<F> GeneratedIdSource for F
where
    F: Fn(GeneratedIdKind) -> Result<String, String> + Send + Sync,
{
    fn generate(&self, kind: GeneratedIdKind) -> Result<String, String> {
        self(kind)
    }
}

pub(super) fn generate_id(
    source: &dyn GeneratedIdSource,
    kind: GeneratedIdKind,
) -> Result<String, PostgresError> {
    source.generate(kind).map_err(|error| {
        PostgresError::internal_message(format!("failed to generate {kind:?}: {error}"))
    })
}

#[cfg(any(test, feature = "local-dev", feature = "test-support"))]
pub(crate) fn test_generated_id(kind: GeneratedIdKind) -> Result<String, String> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    Ok(match kind {
        GeneratedIdKind::CleanupGeneration => format!("test_cleanup_{sequence:016x}"),
        GeneratedIdKind::OutboxJob => format!("outbox_test_{sequence:016x}"),
    })
}
