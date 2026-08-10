use super::{ObjectStore, ensure_object_size};
use crate::ObjectStoreError;
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use std::sync::Arc;

const ENCRYPTED_OBJECT_MAGIC: &[u8] = b"scope-vcs-object-v1\n";
const ENCRYPTED_OBJECT_NONCE_BYTES: usize = 12;
const ENCRYPTED_OBJECT_TAG_BYTES: usize = 16;

pub struct EncryptedObjectStore {
    inner: Arc<dyn ObjectStore>,
    key: [u8; 32],
    previous_key: Option<[u8; 32]>,
}

impl EncryptedObjectStore {
    pub fn new(inner: Arc<dyn ObjectStore>, key: [u8; 32]) -> Self {
        Self {
            inner,
            key,
            previous_key: None,
        }
    }

    pub fn with_previous_key(
        inner: Arc<dyn ObjectStore>,
        key: [u8; 32],
        previous_key: [u8; 32],
    ) -> Self {
        Self {
            inner,
            key,
            previous_key: Some(previous_key),
        }
    }

    fn cipher(key: &[u8; 32]) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new(Key::from_slice(key))
    }

    fn decrypt_envelope_with_key(
        encryption_key: &[u8; 32],
        object_key: &str,
        envelope: &[u8],
    ) -> Result<Vec<u8>, ObjectStoreError> {
        let Some(payload) = envelope.strip_prefix(ENCRYPTED_OBJECT_MAGIC) else {
            return Err(ObjectStoreError::integrity(format!(
                "object {object_key} is missing encryption envelope"
            )));
        };
        if payload.len() < ENCRYPTED_OBJECT_NONCE_BYTES {
            return Err(ObjectStoreError::integrity(format!(
                "object {object_key} has an invalid encryption envelope"
            )));
        }
        let (nonce, ciphertext) = payload.split_at(ENCRYPTED_OBJECT_NONCE_BYTES);
        Self::cipher(encryption_key)
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: object_key.as_bytes(),
                },
            )
            .map_err(|_| {
                ObjectStoreError::integrity(format!("object {object_key} failed decryption"))
            })
    }

    fn decrypt_envelope(
        &self,
        object_key: &str,
        envelope: &[u8],
    ) -> Result<Vec<u8>, ObjectStoreError> {
        Self::decrypt_envelope_with_key(&self.key, object_key, envelope).or_else(|current_error| {
            let Some(previous_key) = &self.previous_key else {
                return Err(current_error);
            };
            Self::decrypt_envelope_with_key(previous_key, object_key, envelope)
        })
    }

    pub fn reencrypt(&self, object_key: &str) -> Result<bool, ObjectStoreError> {
        let envelope = self.inner.get(object_key)?;
        if Self::decrypt_envelope_with_key(&self.key, object_key, &envelope).is_ok() {
            return Ok(false);
        }
        let previous_key = self.previous_key.as_ref().ok_or_else(|| {
            ObjectStoreError::integrity(format!(
                "object {object_key} is not encrypted with the current key"
            ))
        })?;
        let plaintext = Self::decrypt_envelope_with_key(previous_key, object_key, &envelope)?;
        ensure_content_address_matches(object_key, &plaintext)?;
        self.put(object_key, &plaintext)?;
        Ok(true)
    }

    fn max_envelope_bytes(max_plaintext_bytes: usize) -> usize {
        ENCRYPTED_OBJECT_MAGIC
            .len()
            .saturating_add(ENCRYPTED_OBJECT_NONCE_BYTES)
            .saturating_add(ENCRYPTED_OBJECT_TAG_BYTES)
            .saturating_add(max_plaintext_bytes)
    }
}

fn ensure_content_address_matches(
    object_key: &str,
    plaintext: &[u8],
) -> Result<(), ObjectStoreError> {
    const SHA256_PREFIXES: [&str; 4] = [
        "objects/blobs/",
        "objects/git-bundles/",
        "objects/git-segments/",
        "objects/git-manifests/",
    ];
    if let Some(expected) = SHA256_PREFIXES
        .iter()
        .find_map(|prefix| object_key.strip_prefix(prefix))
    {
        if expected == hex::encode(Sha256::digest(plaintext)) {
            return Ok(());
        }
    } else if let Some(expected) = object_key.strip_prefix("git-blobs/") {
        let mut hasher = Sha1::new();
        hasher.update(format!("blob {}\0", plaintext.len()).as_bytes());
        hasher.update(plaintext);
        if expected == hex::encode(hasher.finalize()) {
            return Ok(());
        }
    }
    Err(ObjectStoreError::integrity(format!(
        "object {object_key} does not match its content address"
    )))
}

impl ObjectStore for EncryptedObjectStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), ObjectStoreError> {
        let mut nonce = [0_u8; ENCRYPTED_OBJECT_NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|error| {
            ObjectStoreError::internal_message(format!("object encryption nonce failed: {error}"))
        })?;
        let ciphertext = Self::cipher(&self.key)
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: bytes,
                    aad: key.as_bytes(),
                },
            )
            .map_err(|_| ObjectStoreError::internal_message("object encryption failed"))?;
        let mut envelope =
            Vec::with_capacity(ENCRYPTED_OBJECT_MAGIC.len() + nonce.len() + ciphertext.len());
        envelope.extend_from_slice(ENCRYPTED_OBJECT_MAGIC);
        envelope.extend_from_slice(&nonce);
        envelope.extend_from_slice(&ciphertext);
        self.inner.put(key, &envelope)
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        let envelope = self.inner.get(key)?;
        self.decrypt_envelope(key, &envelope)
    }

    fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, ObjectStoreError> {
        let envelope = self
            .inner
            .get_bounded(key, Self::max_envelope_bytes(max_bytes))?;
        let bytes = self.decrypt_envelope(key, &envelope)?;
        ensure_object_size("read", key, bytes.len(), max_bytes)?;
        Ok(bytes)
    }

    fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        self.inner.delete(key)
    }

    fn readiness_check(&self) -> Result<(), ObjectStoreError> {
        self.inner.readiness_check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryObjectStore;

    #[test]
    fn encrypted_store_put_get_delete_round_trips_without_plaintext_storage() {
        let raw = Arc::new(MemoryObjectStore::new());
        let encrypted = EncryptedObjectStore::new(raw.clone(), [7_u8; 32]);
        let key = format!(
            "tests/encrypted-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        encrypted.put(&key, b"private source").unwrap();

        let stored = raw.get(&key).unwrap();
        assert_ne!(stored, b"private source");
        assert!(!String::from_utf8_lossy(&stored).contains("private source"));
        assert_eq!(encrypted.get(&key).unwrap(), b"private source");

        encrypted.delete(&key).unwrap();
        assert!(raw.get(&key).is_err());
        assert!(encrypted.get(&key).is_err());
    }

    #[test]
    fn encrypted_store_reencrypts_previous_key_objects_idempotently() {
        let raw = Arc::new(MemoryObjectStore::new());
        let old = EncryptedObjectStore::new(raw.clone(), [3_u8; 32]);
        let plaintext = b"private source";
        let object_key = format!("objects/blobs/{}", hex::encode(Sha256::digest(plaintext)));
        old.put(&object_key, plaintext).unwrap();

        let rotating = EncryptedObjectStore::with_previous_key(raw.clone(), [7_u8; 32], [3_u8; 32]);
        assert_eq!(rotating.get(&object_key).unwrap(), plaintext);
        assert!(rotating.reencrypt(&object_key).unwrap());
        assert!(!rotating.reencrypt(&object_key).unwrap());

        let current = EncryptedObjectStore::new(raw.clone(), [7_u8; 32]);
        assert_eq!(current.get(&object_key).unwrap(), plaintext);
        assert!(old.get(&object_key).is_err());
    }

    #[test]
    fn encrypted_store_refuses_to_reencrypt_non_content_addressed_objects() {
        let raw = Arc::new(MemoryObjectStore::new());
        let old = EncryptedObjectStore::new(raw.clone(), [3_u8; 32]);
        let object_key = "objects/blobs/not-the-content-hash";
        old.put(object_key, b"private source").unwrap();
        let rotating = EncryptedObjectStore::with_previous_key(raw.clone(), [7_u8; 32], [3_u8; 32]);

        let error = rotating.reencrypt(object_key).unwrap_err();

        assert_eq!(error.kind, crate::ObjectStoreErrorKind::Integrity);
        assert!(error.message.contains("does not match its content address"));
        assert_eq!(old.get(object_key).unwrap(), b"private source");
    }
}
