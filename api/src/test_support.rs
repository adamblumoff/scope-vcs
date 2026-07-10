use crate::{AppState, object_store::ObjectStore};
use axum::Router;
use std::sync::Arc;

pub struct TestApp {
    state: AppState,
}

impl TestApp {
    pub fn new() -> Self {
        Self {
            state: AppState::test_state(),
        }
    }

    pub fn with_unavailable_object_store(mut self) -> Self {
        self.state.object_store = Arc::new(UnavailableObjectStore);
        self
    }

    pub fn router(&self) -> Router {
        crate::router(self.state.clone())
    }
}

impl Default for TestApp {
    fn default() -> Self {
        Self::new()
    }
}

struct UnavailableObjectStore;

impl ObjectStore for UnavailableObjectStore {
    fn put(&self, _key: &str, _bytes: &[u8]) -> Result<(), scope_core::error::ApiError> {
        Ok(())
    }

    fn get(&self, _key: &str) -> Result<Vec<u8>, scope_core::error::ApiError> {
        Ok(Vec::new())
    }

    fn delete(&self, _key: &str) -> Result<(), scope_core::error::ApiError> {
        Ok(())
    }

    fn readiness_check(&self) -> Result<(), scope_core::error::ApiError> {
        Err(scope_core::error::ApiError::service_unavailable(
            "secret internal object-store hostname is unavailable",
        ))
    }
}
