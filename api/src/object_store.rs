use crate::{
    config::{
        SCOPE_BUCKET_ACCESS_KEY_ID_ENV, SCOPE_BUCKET_ENDPOINT_ENV,
        SCOPE_BUCKET_FORCE_PATH_STYLE_ENV, SCOPE_BUCKET_NAME_ENV, SCOPE_BUCKET_REGION_ENV,
        SCOPE_BUCKET_SECRET_ACCESS_KEY_ENV, SCOPE_OBJECT_ENCRYPTION_KEY_ENV, non_empty_env,
    },
    domain::store::SourceBlob,
    error::ApiError,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use hmac::{Hmac, Mac};
use reqwest::blocking::Client;
use sha1::{Digest as Sha1Digest, Sha1};
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};
use time::OffsetDateTime;

type HmacSha256 = Hmac<Sha256>;
const ENCRYPTED_OBJECT_MAGIC: &[u8] = b"scope-vcs-object-v1\n";
const ENCRYPTED_OBJECT_NONCE_BYTES: usize = 12;

pub trait ObjectStore: Send + Sync {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), ApiError>;
    fn get(&self, key: &str) -> Result<Vec<u8>, ApiError>;
    fn delete(&self, key: &str) -> Result<(), ApiError>;
}

#[derive(Clone)]
pub struct MemoryObjectStore {
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

impl MemoryObjectStore {
    pub fn new() -> Self {
        static OBJECTS: OnceLock<Arc<Mutex<BTreeMap<String, Vec<u8>>>>> = OnceLock::new();
        Self {
            objects: OBJECTS
                .get_or_init(|| Arc::new(Mutex::new(BTreeMap::new())))
                .clone(),
        }
    }

    #[cfg(test)]
    pub fn contains_key(&self, key: &str) -> bool {
        self.objects
            .lock()
            .expect("object store lock")
            .contains_key(key)
    }

    #[cfg(test)]
    pub fn contains_bytes(&self, bytes: &[u8]) -> bool {
        self.objects
            .lock()
            .expect("object store lock")
            .values()
            .any(|stored| stored == bytes)
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

pub(crate) struct EncryptedObjectStore {
    inner: Arc<dyn ObjectStore>,
    key: [u8; 32],
}

impl EncryptedObjectStore {
    pub(crate) fn from_env(inner: Arc<dyn ObjectStore>) -> anyhow::Result<Self> {
        let encoded = required_env(SCOPE_OBJECT_ENCRYPTION_KEY_ENV)?;
        let decoded = BASE64.decode(encoded.trim()).map_err(|error| {
            anyhow::anyhow!("{SCOPE_OBJECT_ENCRYPTION_KEY_ENV} must be base64: {error}")
        })?;
        let key = decoded.as_slice().try_into().map_err(|_| {
            anyhow::anyhow!("{SCOPE_OBJECT_ENCRYPTION_KEY_ENV} must decode to exactly 32 bytes")
        })?;
        Ok(Self::new(inner, key))
    }

    pub(crate) fn new(inner: Arc<dyn ObjectStore>, key: [u8; 32]) -> Self {
        Self { inner, key }
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new(Key::from_slice(&self.key))
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

    fn delete(&self, key: &str) -> Result<(), ApiError> {
        self.inner.delete(key)
    }
}

pub(crate) struct S3ObjectStore {
    client: Option<Client>,
    endpoint: String,
    bucket: String,
    region: String,
    access_key_id: String,
    secret_access_key: String,
    force_path_style: bool,
}

impl S3ObjectStore {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let endpoint = required_env(SCOPE_BUCKET_ENDPOINT_ENV)?;
        let bucket = required_env(SCOPE_BUCKET_NAME_ENV)?;
        let region = required_env(SCOPE_BUCKET_REGION_ENV)?;
        let access_key_id = required_env(SCOPE_BUCKET_ACCESS_KEY_ID_ENV)?;
        let secret_access_key = required_env(SCOPE_BUCKET_SECRET_ACCESS_KEY_ENV)?;
        let force_path_style = non_empty_env(SCOPE_BUCKET_FORCE_PATH_STYLE_ENV)
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);

        Ok(Self {
            client: Some(Client::new()),
            endpoint: endpoint.trim_end_matches('/').to_string(),
            bucket,
            region,
            access_key_id,
            secret_access_key,
            force_path_style,
        })
    }

    fn request_url(&self, key: &str) -> String {
        if self.force_path_style {
            format!("{}/{}/{}", self.endpoint, self.bucket, key)
        } else {
            let scheme_end = self
                .endpoint
                .find("://")
                .map(|index| index + 3)
                .unwrap_or(0);
            let (scheme, host) = self.endpoint.split_at(scheme_end);
            format!("{scheme}{}.{}", self.bucket, host.trim_start_matches('/')) + "/" + key
        }
    }

    fn canonical_uri(&self, key: &str) -> String {
        if self.force_path_style {
            format!("/{}/{}", self.bucket, key)
        } else {
            format!("/{key}")
        }
    }

    fn signed_headers(
        &self,
        method: &str,
        key: &str,
        payload: &[u8],
    ) -> Result<Vec<(String, String)>, ApiError> {
        let now = OffsetDateTime::now_utc();
        let amz_date = format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
            now.year(),
            u8::from(now.month()),
            now.day(),
            now.hour(),
            now.minute(),
            now.second()
        );
        let date_stamp = &amz_date[..8];
        let payload_hash = hex::encode(Sha256::digest(payload));
        let host = self
            .request_url(key)
            .split("://")
            .nth(1)
            .and_then(|value| value.split('/').next())
            .ok_or_else(|| ApiError::internal_message("invalid bucket endpoint"))?
            .to_string();
        let canonical_headers =
            format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n");
        let signed_headers = "host;x-amz-content-sha256;x-amz-date";
        let canonical_request = format!(
            "{method}\n{}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
            self.canonical_uri(key)
        );
        let credential_scope = format!("{date_stamp}/{}/s3/aws4_request", self.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            hex::encode(Sha256::digest(canonical_request.as_bytes()))
        );
        let signing_key = signing_key(&self.secret_access_key, date_stamp, &self.region)?;
        let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes())?);
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.access_key_id
        );

        Ok(vec![
            ("authorization".to_string(), authorization),
            ("host".to_string(), host),
            ("x-amz-content-sha256".to_string(), payload_hash),
            ("x-amz-date".to_string(), amz_date),
        ])
    }

    fn send(&self, method: &str, key: &str, payload: Vec<u8>) -> Result<Vec<u8>, ApiError> {
        let url = self.request_url(key);
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ApiError::internal_message("object store client is shut down"))?;
        let mut request = match method {
            "GET" => client.get(&url),
            "PUT" => client.put(&url).body(payload.clone()),
            "DELETE" => client.delete(&url),
            _ => {
                return Err(ApiError::internal_message(
                    "unsupported object store method",
                ));
            }
        };
        for (name, value) in self.signed_headers(method, key, &payload)? {
            request = request.header(name, value);
        }
        send_blocking_request(method, key, request)
    }
}

fn send_blocking_request(
    method: &str,
    key: &str,
    request: reqwest::blocking::RequestBuilder,
) -> Result<Vec<u8>, ApiError> {
    let send = || {
        let response = request.send().map_err(ApiError::internal)?;
        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::service_unavailable(format!(
                "object store {method} failed for {key}: {status}"
            )));
        }
        response
            .bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(ApiError::internal)
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(send)
    } else {
        send()
    }
}

impl Drop for S3ObjectStore {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            // reqwest's blocking client owns runtime resources. This object is
            // process-lifetime state, so avoid async-context shutdown panics.
            std::mem::forget(client);
        }
    }
}

impl ObjectStore for S3ObjectStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), ApiError> {
        self.send("PUT", key, bytes.to_vec()).map(|_| ())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ApiError> {
        self.send("GET", key, Vec::new())
    }

    fn delete(&self, key: &str) -> Result<(), ApiError> {
        self.send("DELETE", key, Vec::new()).map(|_| ())
    }
}

pub fn repo_object_for_bytes(kind: &str, object_id: &str, bytes: &[u8]) -> SourceBlob {
    let sha256 = hex::encode(Sha256::digest(bytes));
    let git_oid = git_blob_oid(bytes);
    SourceBlob {
        object_key: format!("objects/{kind}/{object_id}"),
        sha256,
        git_oid,
        size_bytes: bytes.len() as u64,
    }
}

pub fn put_source_blob(
    store: &dyn ObjectStore,
    _repo_id: &str,
    bytes: &[u8],
) -> Result<SourceBlob, ApiError> {
    put_repo_object(store, _repo_id, "blobs", bytes)
}

pub fn put_repo_object(
    store: &dyn ObjectStore,
    _repo_id: &str,
    kind: &str,
    bytes: &[u8],
) -> Result<SourceBlob, ApiError> {
    let object_id = random_object_id()?;
    let blob = repo_object_for_bytes(kind, &object_id, bytes);
    store.put(&blob.object_key, bytes)?;
    Ok(blob)
}

pub fn source_blob_text(store: &dyn ObjectStore, blob: &SourceBlob) -> Result<String, ApiError> {
    let bytes = source_blob_bytes(store, blob)?;
    String::from_utf8(bytes).map_err(ApiError::bad_request)
}

pub fn source_blob_bytes(store: &dyn ObjectStore, blob: &SourceBlob) -> Result<Vec<u8>, ApiError> {
    let bytes = store.get(&blob.object_key)?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    if sha256 != blob.sha256 {
        return Err(ApiError::internal_message(format!(
            "object {} failed sha256 verification",
            blob.object_key
        )));
    }
    Ok(bytes)
}

pub fn delete_source_blobs<'a>(
    store: &dyn ObjectStore,
    blobs: impl IntoIterator<Item = &'a SourceBlob>,
) -> Result<(), ApiError> {
    let mut keys = blobs
        .into_iter()
        .map(|blob| blob.object_key.as_str())
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        store.delete(key)?;
    }
    Ok(())
}

fn required_env(name: &str) -> anyhow::Result<String> {
    non_empty_env(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

fn random_object_id() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("object key generation failed: {error}"))
    })?;
    Ok(hex::encode(bytes))
}

fn git_blob_oid(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn signing_key(secret: &str, date: &str, region: &str) -> Result<Vec<u8>, ApiError> {
    let date_key = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes())?;
    let region_key = hmac_sha256(&date_key, region.as_bytes())?;
    let service_key = hmac_sha256(&region_key, b"s3")?;
    hmac_sha256(&service_key, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>, ApiError> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).map_err(ApiError::internal)?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_store_persists_ciphertext_and_returns_plaintext() {
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
    }
}
