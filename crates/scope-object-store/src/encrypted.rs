use super::{ObjectStore, ensure_object_size};
use crate::ObjectStoreError;
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce, Tag,
    aead::{AeadInPlace, KeyInit},
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
    pub fn new(inner: Arc<dyn ObjectStore>, key: [u8; 32]) -> Self {
        Self { inner, key }
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new(Key::from_slice(&self.key))
    }

    fn decrypt_envelope(
        &self,
        key: &str,
        mut envelope: Vec<u8>,
    ) -> Result<Vec<u8>, ObjectStoreError> {
        if !envelope.starts_with(ENCRYPTED_OBJECT_MAGIC) {
            return Err(ObjectStoreError::integrity(format!(
                "object {key} is missing encryption envelope"
            )));
        }
        let header_bytes = ENCRYPTED_OBJECT_MAGIC.len() + ENCRYPTED_OBJECT_NONCE_BYTES;
        if envelope.len() < header_bytes + ENCRYPTED_OBJECT_TAG_BYTES {
            return Err(ObjectStoreError::integrity(format!(
                "object {key} has an invalid encryption envelope"
            )));
        }
        let nonce = Nonce::clone_from_slice(&envelope[ENCRYPTED_OBJECT_MAGIC.len()..header_bytes]);
        let tag_start = envelope.len() - ENCRYPTED_OBJECT_TAG_BYTES;
        let tag = Tag::clone_from_slice(&envelope[tag_start..]);
        self.cipher()
            .decrypt_in_place_detached(
                &nonce,
                key.as_bytes(),
                &mut envelope[header_bytes..tag_start],
                &tag,
            )
            .map_err(|_| ObjectStoreError::integrity(format!("object {key} failed decryption")))?;
        envelope.copy_within(header_bytes..tag_start, 0);
        envelope.truncate(tag_start - header_bytes);
        Ok(envelope)
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
    fn put(&self, key: &str, mut bytes: Vec<u8>) -> Result<(), ObjectStoreError> {
        let mut nonce = [0_u8; ENCRYPTED_OBJECT_NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|error| {
            ObjectStoreError::internal_message(format!("object encryption nonce failed: {error}"))
        })?;
        let header_bytes = ENCRYPTED_OBJECT_MAGIC.len() + nonce.len();
        let plaintext_bytes = bytes.len();
        bytes.reserve_exact(header_bytes + ENCRYPTED_OBJECT_TAG_BYTES);
        bytes.resize(plaintext_bytes + header_bytes, 0);
        bytes.copy_within(..plaintext_bytes, header_bytes);
        bytes[..ENCRYPTED_OBJECT_MAGIC.len()].copy_from_slice(ENCRYPTED_OBJECT_MAGIC);
        bytes[ENCRYPTED_OBJECT_MAGIC.len()..header_bytes].copy_from_slice(&nonce);
        let tag = self
            .cipher()
            .encrypt_in_place_detached(
                Nonce::from_slice(&nonce),
                key.as_bytes(),
                &mut bytes[header_bytes..],
            )
            .map_err(|_| ObjectStoreError::internal_message("object encryption failed"))?;
        bytes.extend_from_slice(&tag);
        self.inner.put(key, bytes)
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        let envelope = self.inner.get(key)?;
        self.decrypt_envelope(key, envelope)
    }

    fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, ObjectStoreError> {
        let envelope = self
            .inner
            .get_bounded(key, Self::max_envelope_bytes(max_bytes))?;
        let bytes = self.decrypt_envelope(key, envelope)?;
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
    fn encryption_preserves_envelope_format_and_rejects_tampering() {
        use chacha20poly1305::aead::{Aead, Payload};
        let raw = Arc::new(MemoryObjectStore::new());
        let encrypted = EncryptedObjectStore::new(raw.clone(), [7_u8; 32]);
        for plaintext in [vec![], b"private source".to_vec(), vec![42; 1024 * 1024]] {
            encrypted.put("source", plaintext.clone()).unwrap();
            let stored = raw.get("source").unwrap();
            let header = ENCRYPTED_OBJECT_MAGIC.len() + ENCRYPTED_OBJECT_NONCE_BYTES;
            let nonce = Nonce::from_slice(&stored[ENCRYPTED_OBJECT_MAGIC.len()..header]);
            assert_eq!(
                encrypted
                    .cipher()
                    .decrypt(
                        nonce,
                        Payload {
                            msg: &stored[header..],
                            aad: b"source"
                        }
                    )
                    .unwrap(),
                plaintext
            );
            assert_eq!(encrypted.get("source").unwrap(), plaintext);
            raw.put("different-key", stored.clone()).unwrap();
            assert_eq!(
                encrypted.get("different-key").unwrap_err().kind,
                crate::ObjectStoreErrorKind::Integrity
            );
            let mut damaged = stored;
            *damaged.last_mut().unwrap() ^= 1;
            raw.put("source", damaged).unwrap();
            assert_eq!(
                encrypted.get("source").unwrap_err().kind,
                crate::ObjectStoreErrorKind::Integrity
            );
        }
    }

    #[test]
    fn bounded_decryption_accepts_the_limit_and_rejects_larger_plaintext() {
        let raw = Arc::new(MemoryObjectStore::new());
        let encrypted = EncryptedObjectStore::new(raw, [7_u8; 32]);
        encrypted.put("source", vec![42; 16]).unwrap();
        assert_eq!(encrypted.get_bounded("source", 16).unwrap(), vec![42; 16]);
        assert_eq!(
            encrypted.get_bounded("source", 15).unwrap_err().kind,
            crate::ObjectStoreErrorKind::PayloadTooLarge
        );
    }

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

        encrypted.put(&key, b"private source".to_vec()).unwrap();

        let stored = raw.get(&key).unwrap();
        assert_ne!(stored, b"private source");
        assert!(!String::from_utf8_lossy(&stored).contains("private source"));
        assert_eq!(encrypted.get(&key).unwrap(), b"private source");

        encrypted.delete(&key).unwrap();
        assert!(raw.get(&key).is_err());
        assert!(encrypted.get(&key).is_err());
    }
}
