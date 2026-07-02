use super::{ObjectStore, ensure_object_size, required_env};
use crate::{config::SCOPE_OBJECT_ENCRYPTION_KEY_ENV, error::ApiError};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use std::sync::Arc;

const ENCRYPTED_OBJECT_MAGIC: &[u8] = b"scope-vcs-object-v1\n";
const ENCRYPTED_OBJECT_NONCE_BYTES: usize = 12;
const ENCRYPTED_OBJECT_TAG_BYTES: usize = 16;

pub struct EncryptedObjectStore {
    inner: Arc<dyn ObjectStore>,
    key: [u8; 32],
}

impl EncryptedObjectStore {
    pub fn from_env(inner: Arc<dyn ObjectStore>) -> anyhow::Result<Self> {
        let encoded = required_env(SCOPE_OBJECT_ENCRYPTION_KEY_ENV)?;
        let decoded = BASE64.decode(encoded.trim()).map_err(|error| {
            anyhow::anyhow!("{SCOPE_OBJECT_ENCRYPTION_KEY_ENV} must be base64: {error}")
        })?;
        let key = decoded.as_slice().try_into().map_err(|_| {
            anyhow::anyhow!("{SCOPE_OBJECT_ENCRYPTION_KEY_ENV} must decode to exactly 32 bytes")
        })?;
        Ok(Self::new(inner, key))
    }

    pub fn new(inner: Arc<dyn ObjectStore>, key: [u8; 32]) -> Self {
        Self { inner, key }
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new(Key::from_slice(&self.key))
    }

    fn decrypt_envelope(&self, key: &str, envelope: Vec<u8>) -> Result<Vec<u8>, ApiError> {
        let Some(payload) = envelope.strip_prefix(ENCRYPTED_OBJECT_MAGIC) else {
            return Err(ApiError::internal_message(format!(
                "object {key} is missing encryption envelope"
            )));
        };
        if payload.len() < ENCRYPTED_OBJECT_NONCE_BYTES {
            return Err(ApiError::internal_message(format!(
                "object {key} has an invalid encryption envelope"
            )));
        }
        let (nonce, ciphertext) = payload.split_at(ENCRYPTED_OBJECT_NONCE_BYTES);
        self.cipher()
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: key.as_bytes(),
                },
            )
            .map_err(|_| ApiError::internal_message(format!("object {key} failed decryption")))
    }

    fn max_envelope_bytes(max_plaintext_bytes: usize) -> usize {
        ENCRYPTED_OBJECT_MAGIC
            .len()
            .saturating_add(ENCRYPTED_OBJECT_NONCE_BYTES)
            .saturating_add(ENCRYPTED_OBJECT_TAG_BYTES)
            .saturating_add(max_plaintext_bytes)
    }
}

impl ObjectStore for EncryptedObjectStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), ApiError> {
        let mut nonce = [0_u8; ENCRYPTED_OBJECT_NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|error| {
            ApiError::internal_message(format!("object encryption nonce failed: {error}"))
        })?;
        let ciphertext = self
            .cipher()
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: bytes,
                    aad: key.as_bytes(),
                },
            )
            .map_err(|_| ApiError::internal_message("object encryption failed"))?;
        let mut envelope =
            Vec::with_capacity(ENCRYPTED_OBJECT_MAGIC.len() + nonce.len() + ciphertext.len());
        envelope.extend_from_slice(ENCRYPTED_OBJECT_MAGIC);
        envelope.extend_from_slice(&nonce);
        envelope.extend_from_slice(&ciphertext);
        self.inner.put(key, &envelope)
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ApiError> {
        let envelope = self.inner.get(key)?;
        self.decrypt_envelope(key, envelope)
    }

    fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, ApiError> {
        let envelope = self
            .inner
            .get_bounded(key, Self::max_envelope_bytes(max_bytes))?;
        let bytes = self.decrypt_envelope(key, envelope)?;
        ensure_object_size("read", key, bytes.len(), max_bytes)?;
        Ok(bytes)
    }

    fn delete(&self, key: &str) -> Result<(), ApiError> {
        self.inner.delete(key)
    }

    fn readiness_check(&self) -> Result<(), ApiError> {
        self.inner.readiness_check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object_store::MemoryObjectStore;

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
}
