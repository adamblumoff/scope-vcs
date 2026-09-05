use crate::AppState;
use axum::Router;
use scope_object_store::{ObjectStore, ObjectStoreError};
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
    fn put(&self, _key: &str, _bytes: Vec<u8>) -> Result<(), ObjectStoreError> {
        Ok(())
    }

    fn get(&self, _key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        Ok(Vec::new())
    }

    fn delete(&self, _key: &str) -> Result<(), ObjectStoreError> {
        Ok(())
    }

    fn readiness_check(&self) -> Result<(), ObjectStoreError> {
        Err(ObjectStoreError::service_unavailable(
            "secret internal object-store hostname is unavailable",
        ))
    }
}
