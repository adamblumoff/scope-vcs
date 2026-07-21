use super::ObjectStore;
use crate::error::ApiError;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

type MemoryObjects = Arc<Mutex<BTreeMap<String, Vec<u8>>>>;

#[derive(Clone)]
pub struct MemoryObjectStore {
    objects: MemoryObjects,
}

impl MemoryObjectStore {
    pub fn new() -> Self {
        Self {
            objects: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn contains_key(&self, key: &str) -> bool {
        self.objects
            .lock()
            .expect("object store lock")
            .contains_key(key)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn contains_bytes(&self, bytes: &[u8]) -> bool {
        self.objects
            .lock()
            .expect("object store lock")
            .values()
            .any(|stored| stored == bytes)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn object_count(&self) -> usize {
        self.objects.lock().expect("object store lock").len()
    }
}

impl Default for MemoryObjectStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectStore for MemoryObjectStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), ApiError> {
        self.objects
            .lock()
            .map_err(|_| ApiError::internal_message("object store lock poisoned"))?
            .insert(key.to_string(), bytes.to_vec());
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ApiError> {
        self.objects
            .lock()
            .map_err(|_| ApiError::internal_message("object store lock poisoned"))?
            .get(key)
            .cloned()
            .ok_or_else(|| ApiError::not_found(format!("object {key} not found")))
    }

    fn delete(&self, key: &str) -> Result<(), ApiError> {
        self.objects
            .lock()
            .map_err(|_| ApiError::internal_message("object store lock poisoned"))?
            .remove(key);
        Ok(())
    }
}
