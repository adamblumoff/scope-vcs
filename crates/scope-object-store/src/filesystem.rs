use super::ObjectStore;
use crate::ObjectStoreError;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileObjectStoreSettings {
    pub root: PathBuf,
}

impl FileObjectStoreSettings {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

pub struct FileObjectStore {
    root: PathBuf,
}

impl FileObjectStore {
    pub fn new(settings: FileObjectStoreSettings) -> Self {
        Self {
            root: settings.root,
        }
    }

    fn path_for_key(&self, key: &str) -> PathBuf {
        let digest = hex::encode(Sha256::digest(key.as_bytes()));
        self.root.join(&digest[..2]).join(digest)
    }

    fn ensure_root(&self) -> Result<(), ObjectStoreError> {
        std::fs::create_dir_all(&self.root).map_err(|error| {
            ObjectStoreError::service_unavailable(format!(
                "failed to prepare local object store {}: {error}",
                self.root.display()
            ))
        })
    }
}

impl ObjectStore for FileObjectStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), ObjectStoreError> {
        let path = self.path_for_key(key);
        let parent = path.parent().ok_or_else(|| {
            ObjectStoreError::internal_message("local object path is missing a parent directory")
        })?;
        std::fs::create_dir_all(parent).map_err(|error| {
            ObjectStoreError::service_unavailable(format!(
                "failed to prepare local object directory {}: {error}",
                parent.display()
            ))
        })?;
        std::fs::write(&path, bytes).map_err(|error| {
            ObjectStoreError::service_unavailable(format!(
                "failed to write local object {}: {error}",
                path.display()
            ))
        })
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        let path = self.path_for_key(key);
        std::fs::read(&path)
            .map_err(|_| ObjectStoreError::not_found(format!("object {key} not found")))
    }

    fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        let path = self.path_for_key(key);
        match std::fs::remove_file(&path) {
            Ok(()) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::remove_dir(parent);
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ObjectStoreError::service_unavailable(format!(
                "failed to delete local object {}: {error}",
                path.display()
            ))),
        }
    }

    fn readiness_check(&self) -> Result<(), ObjectStoreError> {
        self.ensure_root()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_round_trips_and_deletes_by_object_key_hash() {
        let root = std::env::temp_dir().join(format!(
            "scope-file-store-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = FileObjectStore::new(FileObjectStoreSettings::new(root.clone()));

        store.put("../not-a-path", b"payload").unwrap();
        assert_eq!(store.get("../not-a-path").unwrap(), b"payload");
        store.delete("../not-a-path").unwrap();
        assert!(store.get("../not-a-path").is_err());

        let _ = std::fs::remove_dir_all(root);
    }
}
