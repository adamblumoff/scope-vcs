pub(crate) use scope_core::persistence::*;

#[cfg(test)]
pub(crate) fn lock_catalog(
    state: &crate::state::AppState,
) -> Result<std::sync::MutexGuard<'_, crate::domain::store::AppCatalog>, scope_core::error::ApiError>
{
    state.metadata.test_catalog()
}
