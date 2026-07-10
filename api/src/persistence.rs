pub(crate) use scope_core::persistence::*;

#[cfg(test)]
pub(crate) fn lock_catalog(
    state: &crate::state::AppState,
) -> Result<crate::db::TestCatalogGuard, scope_core::error::ApiError> {
    state.metadata.test_catalog()
}
