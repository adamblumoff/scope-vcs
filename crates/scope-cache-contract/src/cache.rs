use scope_cache_domain::{CacheDigest, UploadLeaseId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RestoreCacheRequest {
    pub identity_digest: CacheDigest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum RestoreCacheResponse {
    Hit {
        object_digest: CacheDigest,
        size_bytes: u64,
        download_url: String,
        expires_at_unix: u64,
    },
    Miss,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrepareCacheUploadRequest {
    pub identity_digest: CacheDigest,
    pub object_digest: CacheDigest,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum PrepareCacheUploadResponse {
    UseObject {
        object_digest: CacheDigest,
        expires_at_unix: u64,
    },
    Upload {
        lease_id: UploadLeaseId,
        upload_url: String,
        upload_headers: BTreeMap<String, String>,
        expires_at_unix: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommitCacheUploadRequest {
    pub lease_id: UploadLeaseId,
    pub object_digest: CacheDigest,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommitCacheUploadResponse {
    pub identity_digest: CacheDigest,
    pub object_digest: CacheDigest,
    pub expires_at_unix: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(value: char) -> CacheDigest {
        CacheDigest::parse(value.to_string().repeat(64)).unwrap()
    }

    #[test]
    fn restore_miss_has_a_small_stable_wire_shape() {
        assert_eq!(
            serde_json::to_value(RestoreCacheResponse::Miss).unwrap(),
            serde_json::json!({ "result": "miss" })
        );
    }

    #[test]
    fn prepare_upload_wire_shape_carries_the_exact_content_claim() {
        let request = PrepareCacheUploadRequest {
            identity_digest: digest('a'),
            object_digest: digest('b'),
            size_bytes: 42,
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["size_bytes"], 42);
        assert_eq!(value["identity_digest"], "a".repeat(64));
        assert_eq!(value["object_digest"], "b".repeat(64));
    }

    #[test]
    fn upload_instructions_include_checksum_bound_request_headers() {
        let response = PrepareCacheUploadResponse::Upload {
            lease_id: UploadLeaseId::parse("lease-1").unwrap(),
            upload_url: "https://objects.example/upload".to_string(),
            upload_headers: BTreeMap::from([
                ("content-length".to_string(), "42".to_string()),
                ("x-amz-meta-scope-sha256".to_string(), "a".repeat(64)),
            ]),
            expires_at_unix: 100,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(
            value["upload_headers"]["x-amz-meta-scope-sha256"],
            "a".repeat(64)
        );
        assert_eq!(value["upload_headers"]["content-length"], "42");
    }
}
