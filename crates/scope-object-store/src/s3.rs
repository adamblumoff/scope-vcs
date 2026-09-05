use super::{ObjectStore, ensure_object_size, object_too_large};
use crate::ObjectStoreError;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use hmac::{Hmac, Mac};
use reqwest::blocking::Client;
use sha2::{Digest as _, Sha256};
use std::{io::Read, time::Duration};
use time::OffsetDateTime;

type HmacSha256 = Hmac<Sha256>;
const S3_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const S3_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct S3ObjectStoreSettings {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub force_path_style: bool,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
}

impl S3ObjectStoreSettings {
    pub fn new(
        endpoint: String,
        bucket: String,
        region: String,
        access_key_id: String,
        secret_access_key: String,
    ) -> Self {
        Self {
            endpoint,
            bucket,
            region,
            access_key_id,
            secret_access_key,
            force_path_style: false,
            connect_timeout: S3_CONNECT_TIMEOUT,
            request_timeout: S3_REQUEST_TIMEOUT,
        }
    }
}

pub struct S3ObjectStore {
    client: Option<Client>,
    endpoint: String,
    bucket: String,
    region: String,
    access_key_id: String,
    secret_access_key: String,
    force_path_style: bool,
    request_timeout: Duration,
}

#[derive(Clone)]
pub struct S3Presigner {
    endpoint: String,
    bucket: String,
    region: String,
    access_key_id: String,
    secret_access_key: String,
    force_path_style: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresignedRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl S3Presigner {
    pub fn new(settings: &S3ObjectStoreSettings) -> Self {
        Self {
            endpoint: settings.endpoint.trim_end_matches('/').to_string(),
            bucket: settings.bucket.clone(),
            region: settings.region.clone(),
            access_key_id: settings.access_key_id.clone(),
            secret_access_key: settings.secret_access_key.clone(),
            force_path_style: settings.force_path_style,
        }
    }

    pub fn presign(
        &self,
        method: &str,
        key: &str,
        expires_seconds: u32,
    ) -> Result<String, ObjectStoreError> {
        Ok(self.presign_request(method, key, expires_seconds, &[])?.url)
    }

    pub fn presign_checksum_bound_put(
        &self,
        key: &str,
        expires_seconds: u32,
        checksum_sha256: &str,
        content_length: u64,
    ) -> Result<PresignedRequest, ObjectStoreError> {
        if checksum_sha256.len() != 64
            || !checksum_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ObjectStoreError::internal_message(
                "checksum must be 64 lowercase hexadecimal characters",
            ));
        }
        let checksum_bytes = hex::decode(checksum_sha256).map_err(|_| {
            ObjectStoreError::internal_message("checksum must be valid hexadecimal")
        })?;
        let checksum_base64 = BASE64.encode(checksum_bytes);
        let content_length = content_length.to_string();
        self.presign_request(
            "PUT",
            key,
            expires_seconds,
            &[
                ("content-length", content_length.as_str()),
                ("x-amz-checksum-sha256", checksum_base64.as_str()),
                ("x-amz-meta-scope-sha256", checksum_sha256),
            ],
        )
    }

    fn presign_request(
        &self,
        method: &str,
        key: &str,
        expires_seconds: u32,
        request_headers: &[(&str, &str)],
    ) -> Result<PresignedRequest, ObjectStoreError> {
        if !matches!(method, "GET" | "HEAD" | "PUT")
            || expires_seconds == 0
            || expires_seconds > 3600
        {
            return Err(ObjectStoreError::internal_message(
                "invalid presigned object request",
            ));
        }
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
        let base = if self.force_path_style {
            format!("{}/{}/{}", self.endpoint, self.bucket, key)
        } else {
            let scheme_end = self
                .endpoint
                .find("://")
                .map(|index| index + 3)
                .unwrap_or(0);
            let (scheme, host) = self.endpoint.split_at(scheme_end);
            format!(
                "{scheme}{}.{}/{}",
                self.bucket,
                host.trim_start_matches('/'),
                key
            )
        };
        let host = S3ObjectStore::request_host(&base)?;
        let canonical_uri = if self.force_path_style {
            format!("/{}/{}", self.bucket, key)
        } else {
            format!("/{key}")
        };
        let mut signed_headers = vec![("host", host.as_str())];
        for (name, value) in request_headers {
            let normalized_name = name.trim();
            if normalized_name.is_empty()
                || normalized_name
                    .bytes()
                    .any(|byte| byte.is_ascii_uppercase())
                || normalized_name == "host"
                || value.trim() != *value
                || value.contains(['\r', '\n'])
            {
                return Err(ObjectStoreError::internal_message(
                    "invalid presigned object request header",
                ));
            }
            signed_headers.push((normalized_name, *value));
        }
        signed_headers.sort_by_key(|(name, _)| *name);
        if signed_headers.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(ObjectStoreError::internal_message(
                "duplicate presigned object request header",
            ));
        }
        let signed_header_names = signed_headers
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(";");
        let canonical_headers = signed_headers
            .iter()
            .map(|(name, value)| format!("{name}:{value}\n"))
            .collect::<String>();
        let credential_scope = format!("{date_stamp}/{}/s3/aws4_request", self.region);
        let mut query = [
            ("X-Amz-Algorithm", "AWS4-HMAC-SHA256".to_string()),
            (
                "X-Amz-Credential",
                format!("{}/{}", self.access_key_id, credential_scope),
            ),
            ("X-Amz-Date", amz_date.clone()),
            ("X-Amz-Expires", expires_seconds.to_string()),
            ("X-Amz-SignedHeaders", signed_header_names.clone()),
        ];
        query.sort_by_key(|(name, _)| *name);
        let canonical_query = query
            .iter()
            .map(|(name, value)| format!("{}={}", aws_encode(name), aws_encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        let canonical_request = format!(
            "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_header_names}\nUNSIGNED-PAYLOAD"
        );
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            hex::encode(Sha256::digest(canonical_request.as_bytes()))
        );
        let signing_key = signing_key(&self.secret_access_key, date_stamp, &self.region)?;
        let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes())?);
        Ok(PresignedRequest {
            url: format!("{base}?{canonical_query}&X-Amz-Signature={signature}"),
            headers: request_headers
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                .collect(),
        })
    }
}

fn aws_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

impl S3ObjectStore {
    pub fn new(settings: S3ObjectStoreSettings) -> Result<Self, ObjectStoreError> {
        Ok(Self {
            client: Some(
                Client::builder()
                    .connect_timeout(settings.connect_timeout)
                    .build()
                    .map_err(|error| {
                        ObjectStoreError::internal(format!("building object store client: {error}"))
                    })?,
            ),
            endpoint: settings.endpoint.trim_end_matches("/").to_string(),
            bucket: settings.bucket,
            region: settings.region,
            access_key_id: settings.access_key_id,
            secret_access_key: settings.secret_access_key,
            force_path_style: settings.force_path_style,
            request_timeout: settings.request_timeout,
        })
    }

    fn bucket_url(&self) -> String {
        if self.force_path_style {
            format!("{}/{}", self.endpoint, self.bucket)
        } else {
            let scheme_end = self
                .endpoint
                .find("://")
                .map(|index| index + 3)
                .unwrap_or(0);
            let (scheme, host) = self.endpoint.split_at(scheme_end);
            format!("{scheme}{}.{}", self.bucket, host.trim_start_matches('/'))
        }
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

    fn bucket_canonical_uri(&self) -> String {
        if self.force_path_style {
            format!("/{}", self.bucket)
        } else {
            "/".to_string()
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
        canonical_uri: &str,
        host: &str,
        payload: &[u8],
    ) -> Result<Vec<(String, String)>, ObjectStoreError> {
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
        let canonical_headers =
            format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n");
        let signed_headers = "host;x-amz-content-sha256;x-amz-date";
        let canonical_request = format!(
            "{method}\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
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
            ("host".to_string(), host.to_string()),
            ("x-amz-content-sha256".to_string(), payload_hash),
            ("x-amz-date".to_string(), amz_date),
        ])
    }

    fn request_host(url: &str) -> Result<String, ObjectStoreError> {
        let url = reqwest::Url::parse(url).map_err(ObjectStoreError::internal)?;
        let host = url
            .host_str()
            .ok_or_else(|| ObjectStoreError::internal_message("invalid bucket endpoint"))?;
        Ok(match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        })
    }

    fn send_bucket(&self, method: &str, payload: Vec<u8>) -> Result<Vec<u8>, ObjectStoreError> {
        let url = self.bucket_url();
        let host = Self::request_host(&url)?;
        let canonical_uri = self.bucket_canonical_uri();
        let client = self.client.as_ref().ok_or_else(|| {
            ObjectStoreError::internal_message("object store client is shut down")
        })?;
        let mut request = match method {
            "HEAD" => client.head(&url).timeout(self.request_timeout),
            _ => {
                return Err(ObjectStoreError::internal_message(
                    "unsupported object store method",
                ));
            }
        };
        for (name, value) in self.signed_headers(method, &canonical_uri, &host, &payload)? {
            request = request.header(name, value);
        }
        send_blocking_request(method, "bucket", request, None)
    }

    fn send(
        &self,
        method: &str,
        key: &str,
        payload: Vec<u8>,
        max_bytes: Option<usize>,
    ) -> Result<Vec<u8>, ObjectStoreError> {
        let url = self.request_url(key);
        let host = Self::request_host(&url)?;
        let canonical_uri = self.canonical_uri(key);
        let client = self.client.as_ref().ok_or_else(|| {
            ObjectStoreError::internal_message("object store client is shut down")
        })?;
        let headers = self.signed_headers(method, &canonical_uri, &host, &payload)?;
        let mut request = match method {
            "GET" => client.get(&url).timeout(self.request_timeout),
            "PUT" => client.put(&url).timeout(self.request_timeout).body(payload),
            "DELETE" => client.delete(&url).timeout(self.request_timeout),
            _ => {
                return Err(ObjectStoreError::internal_message(
                    "unsupported object store method",
                ));
            }
        };
        for (name, value) in headers {
            request = request.header(name, value);
        }
        send_blocking_request(method, key, request, max_bytes)
    }
}

fn send_blocking_request(
    method: &str,
    key: &str,
    request: reqwest::blocking::RequestBuilder,
    max_bytes: Option<usize>,
) -> Result<Vec<u8>, ObjectStoreError> {
    let send = || {
        let response = request.send().map_err(ObjectStoreError::internal)?;
        let status = response.status();
        if !status.is_success() {
            return Err(ObjectStoreError::service_unavailable(format!(
                "object store {method} failed for {key}: {status}"
            )));
        }
        read_response_body(response, key, max_bytes)
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(send)
    } else {
        send()
    }
}

fn read_response_body(
    mut response: reqwest::blocking::Response,
    key: &str,
    max_bytes: Option<usize>,
) -> Result<Vec<u8>, ObjectStoreError> {
    let Some(max_bytes) = max_bytes else {
        let mut body = Vec::new();
        response
            .read_to_end(&mut body)
            .map_err(ObjectStoreError::internal)?;
        return Ok(body);
    };

    if let Some(content_length) = response.content_length()
        && content_length > max_bytes as u64
    {
        return Err(object_too_large(
            "read",
            key,
            usize::try_from(content_length).unwrap_or(usize::MAX),
            max_bytes,
        ));
    }

    let mut body = Vec::new();
    response
        .take((max_bytes as u64).saturating_add(1))
        .read_to_end(&mut body)
        .map_err(ObjectStoreError::internal)?;
    ensure_object_size("read", key, body.len(), max_bytes)?;
    Ok(body)
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
    fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), ObjectStoreError> {
        self.send("PUT", key, bytes, None).map(|_| ())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        self.send("GET", key, Vec::new(), None)
    }

    fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, ObjectStoreError> {
        self.send("GET", key, Vec::new(), Some(max_bytes))
    }

    fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        self.send("DELETE", key, Vec::new(), None).map(|_| ())
    }

    fn readiness_check(&self) -> Result<(), ObjectStoreError> {
        self.send_bucket("HEAD", Vec::new()).map(|_| ())
    }
}

fn signing_key(secret: &str, date: &str, region: &str) -> Result<Vec<u8>, ObjectStoreError> {
    let date_key = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes())?;
    let region_key = hmac_sha256(&date_key, region.as_bytes())?;
    let service_key = hmac_sha256(&region_key, b"s3")?;
    hmac_sha256(&service_key, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>, ObjectStoreError> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).map_err(ObjectStoreError::internal)?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn s3_store_checks_bucket_with_signed_head_request() {
        let server = TestS3Server::start(vec![(vec![], None)]);
        let store = test_s3_store(&server.endpoint);

        store.readiness_check().unwrap();

        let request = server.recv();
        assert_eq!(request.method, "HEAD");
        assert_eq!(request.path, "/scope-bucket");
        assert_eq!(
            request.headers.get("host").map(String::as_str),
            Some(server.host.as_str())
        );
        assert_signed_s3_headers(&request);
    }

    #[test]
    fn s3_store_put_get_delete_use_signed_local_s3_compatible_requests() {
        let server = TestS3Server::start(vec![
            (vec![], None),
            (b"stored payload".to_vec(), None),
            (vec![], None),
        ]);
        let store = test_s3_store(&server.endpoint);
        let key = "objects/blob-1";

        store.put(key, b"stored payload".to_vec()).unwrap();
        assert_eq!(store.get(key).unwrap(), b"stored payload");
        store.delete(key).unwrap();

        for (method, body) in [
            ("PUT", b"stored payload".as_slice()),
            ("GET", b"".as_slice()),
            ("DELETE", b"".as_slice()),
        ] {
            let request = server.recv();
            assert_eq!(request.method, method);
            assert_eq!(request.path, "/scope-bucket/objects/blob-1");
            assert_eq!(request.body, body);
            assert_signed_s3_headers(&request);
        }
    }

    #[test]
    fn s3_store_bounded_get_rejects_declared_oversized_body_before_reading() {
        let server = TestS3Server::start(vec![(vec![], Some(5))]);
        let store = test_s3_store(&server.endpoint);

        let error = store.get_bounded("objects/too-large", 4).unwrap_err();

        assert_eq!(error.kind, crate::ObjectStoreErrorKind::PayloadTooLarge);
        assert!(error.message.contains("exceeds 4 bytes"));
        let request = server.recv();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/scope-bucket/objects/too-large");
        assert_signed_s3_headers(&request);
    }

    #[test]
    fn request_host_excludes_query_strings_from_virtual_host_bucket_urls() {
        assert_eq!(
            S3ObjectStore::request_host("https://scope-bucket.storage.example?list-type=2")
                .unwrap(),
            "scope-bucket.storage.example"
        );
    }

    #[test]
    fn presigner_produces_bounded_path_style_urls_without_exposing_the_secret() {
        let mut settings = S3ObjectStoreSettings::new(
            "https://storage.example".into(),
            "scope-bucket".into(),
            "us-test-1".into(),
            "test-access".into(),
            "test-secret".into(),
        );
        settings.force_path_style = true;
        let url = S3Presigner::new(&settings)
            .presign("PUT", "repos/repo-1/objects/sha256/abc", 900)
            .unwrap();

        assert!(
            url.starts_with(
                "https://storage.example/scope-bucket/repos/repo-1/objects/sha256/abc?"
            )
        );
        assert!(url.contains("X-Amz-Expires=900"));
        assert!(url.contains("X-Amz-Credential=test-access%2F"));
        assert!(url.contains("X-Amz-Signature="));
        assert!(!url.contains("test-secret"));
        assert!(
            S3Presigner::new(&settings)
                .presign("DELETE", "key", 900)
                .is_err()
        );
        assert!(
            S3Presigner::new(&settings)
                .presign("GET", "key", 3_601)
                .is_err()
        );
    }

    #[test]
    fn checksum_bound_put_signs_and_returns_the_required_metadata_header() {
        let mut settings = S3ObjectStoreSettings::new(
            "https://storage.example".into(),
            "scope-cache".into(),
            "us-east-1".into(),
            "access".into(),
            "secret".into(),
        );
        settings.force_path_style = true;
        let checksum = "a".repeat(64);
        let request = S3Presigner::new(&settings)
            .presign_checksum_bound_put("repos/repo-1/objects/sha256/abc", 900, &checksum, 42)
            .unwrap();

        assert!(request.url.contains(
            "X-Amz-SignedHeaders=content-length%3Bhost%3Bx-amz-checksum-sha256%3Bx-amz-meta-scope-sha256"
        ));
        assert_eq!(
            request.headers,
            vec![
                ("content-length".to_string(), "42".to_string()),
                (
                    "x-amz-checksum-sha256".to_string(),
                    "qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqo=".to_string(),
                ),
                ("x-amz-meta-scope-sha256".to_string(), checksum),
            ]
        );
        assert!(!request.url.contains("secret"));
    }

    #[test]
    fn presigner_supports_head_and_rejects_invalid_checksums() {
        let settings = S3ObjectStoreSettings::new(
            "https://storage.example".into(),
            "scope-cache".into(),
            "us-east-1".into(),
            "access".into(),
            "secret".into(),
        );
        let signer = S3Presigner::new(&settings);

        assert!(signer.presign("HEAD", "object", 60).is_ok());
        assert!(
            signer
                .presign_checksum_bound_put("object", 60, "not-a-checksum", 42)
                .is_err()
        );
    }

    fn test_s3_store(endpoint: &str) -> S3ObjectStore {
        S3ObjectStore {
            client: Some(
                Client::builder()
                    .connect_timeout(Duration::from_secs(1))
                    .timeout(Duration::from_secs(1))
                    .build()
                    .unwrap(),
            ),
            endpoint: endpoint.to_string(),
            bucket: "scope-bucket".to_string(),
            region: "us-test-1".to_string(),
            access_key_id: "test-access".to_string(),
            secret_access_key: "test-secret".to_string(),
            force_path_style: true,
            request_timeout: Duration::from_secs(1),
        }
    }

    fn assert_signed_s3_headers(request: &CapturedRequest) {
        let authorization = request
            .headers
            .get("authorization")
            .expect("authorization header");
        assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=test-access/"));
        assert!(authorization.contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date"));
        assert!(!authorization.contains("test-secret"));
        assert!(request.headers.contains_key("x-amz-content-sha256"));
        assert!(request.headers.contains_key("x-amz-date"));
    }

    #[derive(Debug)]
    struct CapturedRequest {
        method: String,
        path: String,
        headers: BTreeMap<String, String>,
        body: Vec<u8>,
    }

    struct TestS3Server {
        endpoint: String,
        host: String,
        requests: std::sync::mpsc::Receiver<CapturedRequest>,
    }

    impl TestS3Server {
        fn start(responses: Vec<(Vec<u8>, Option<usize>)>) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let host = format!("127.0.0.1:{}", addr.port());
            let endpoint = format!("http://{host}");
            let (sender, requests) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                for (body, declared_length) in responses {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_request(&mut stream);
                    sender.send(request).unwrap();
                    let content_length = declared_length.unwrap_or(body.len());
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        content_length
                    );
                    use std::io::Write as _;
                    stream.write_all(headers.as_bytes()).unwrap();
                    stream.write_all(&body).unwrap();
                }
            });
            Self {
                endpoint,
                host,
                requests,
            }
        }

        fn recv(&self) -> CapturedRequest {
            self.requests
                .recv_timeout(Duration::from_secs(2))
                .expect("mock S3 request")
        }
    }

    fn read_request(stream: &mut std::net::TcpStream) -> CapturedRequest {
        use std::io::{BufRead as _, Read as _};

        let mut reader = std::io::BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut request_parts = line.split_whitespace();
        let method = request_parts.next().unwrap().to_string();
        let path = request_parts.next().unwrap().to_string();
        let mut headers = BTreeMap::new();
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
        }
        let content_length = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).unwrap();

        CapturedRequest {
            method,
            path,
            headers,
            body,
        }
    }
}
