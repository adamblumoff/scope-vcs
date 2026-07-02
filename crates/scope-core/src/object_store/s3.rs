use super::{ObjectStore, ensure_object_size, object_too_large, required_env};
use crate::{
    config::{
        SCOPE_BUCKET_ACCESS_KEY_ID_ENV, SCOPE_BUCKET_ENDPOINT_ENV,
        SCOPE_BUCKET_FORCE_PATH_STYLE_ENV, SCOPE_BUCKET_NAME_ENV, SCOPE_BUCKET_REGION_ENV,
        SCOPE_BUCKET_SECRET_ACCESS_KEY_ENV, non_empty_env,
    },
    error::ApiError,
};
use hmac::{Hmac, Mac};
use reqwest::blocking::Client;
use sha2::{Digest as _, Sha256};
use std::{io::Read, time::Duration};
use time::OffsetDateTime;

type HmacSha256 = Hmac<Sha256>;
const S3_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const S3_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct S3ObjectStore {
    client: Option<Client>,
    endpoint: String,
    bucket: String,
    region: String,
    access_key_id: String,
    secret_access_key: String,
    force_path_style: bool,
}

impl S3ObjectStore {
    pub fn from_env() -> anyhow::Result<Self> {
        let endpoint = required_env(SCOPE_BUCKET_ENDPOINT_ENV)?;
        let bucket = required_env(SCOPE_BUCKET_NAME_ENV)?;
        let region = required_env(SCOPE_BUCKET_REGION_ENV)?;
        let access_key_id = required_env(SCOPE_BUCKET_ACCESS_KEY_ID_ENV)?;
        let secret_access_key = required_env(SCOPE_BUCKET_SECRET_ACCESS_KEY_ENV)?;
        let force_path_style = non_empty_env(SCOPE_BUCKET_FORCE_PATH_STYLE_ENV)
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);

        Ok(Self {
            client: Some(
                Client::builder()
                    .connect_timeout(S3_CONNECT_TIMEOUT)
                    .build()
                    .map_err(|error| anyhow::anyhow!("building object store client: {error}"))?,
            ),
            endpoint: endpoint.trim_end_matches('/').to_string(),
            bucket,
            region,
            access_key_id,
            secret_access_key,
            force_path_style,
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

    fn request_host(url: &str) -> Result<String, ApiError> {
        url.split("://")
            .nth(1)
            .and_then(|value| value.split('/').next())
            .map(ToString::to_string)
            .ok_or_else(|| ApiError::internal_message("invalid bucket endpoint"))
    }

    fn send_bucket(&self, method: &str, payload: Vec<u8>) -> Result<Vec<u8>, ApiError> {
        let url = self.bucket_url();
        let host = Self::request_host(&url)?;
        let canonical_uri = self.bucket_canonical_uri();
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ApiError::internal_message("object store client is shut down"))?;
        let mut request = match method {
            "HEAD" => client.head(&url).timeout(S3_REQUEST_TIMEOUT),
            _ => {
                return Err(ApiError::internal_message(
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
    ) -> Result<Vec<u8>, ApiError> {
        let url = self.request_url(key);
        let host = Self::request_host(&url)?;
        let canonical_uri = self.canonical_uri(key);
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ApiError::internal_message("object store client is shut down"))?;
        let mut request = match method {
            "GET" => client.get(&url).timeout(S3_REQUEST_TIMEOUT),
            "PUT" => client
                .put(&url)
                .timeout(S3_REQUEST_TIMEOUT)
                .body(payload.clone()),
            "DELETE" => client.delete(&url).timeout(S3_REQUEST_TIMEOUT),
            _ => {
                return Err(ApiError::internal_message(
                    "unsupported object store method",
                ));
            }
        };
        for (name, value) in self.signed_headers(method, &canonical_uri, &host, &payload)? {
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
) -> Result<Vec<u8>, ApiError> {
    let send = || {
        let response = request.send().map_err(ApiError::internal)?;
        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::service_unavailable(format!(
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
    response: reqwest::blocking::Response,
    key: &str,
    max_bytes: Option<usize>,
) -> Result<Vec<u8>, ApiError> {
    let Some(max_bytes) = max_bytes else {
        return response
            .bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(ApiError::internal);
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
        .map_err(ApiError::internal)?;
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
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), ApiError> {
        self.send("PUT", key, bytes.to_vec(), None).map(|_| ())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ApiError> {
        self.send("GET", key, Vec::new(), None)
    }

    fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, ApiError> {
        self.send("GET", key, Vec::new(), Some(max_bytes))
    }

    fn delete(&self, key: &str) -> Result<(), ApiError> {
        self.send("DELETE", key, Vec::new(), None).map(|_| ())
    }

    fn readiness_check(&self) -> Result<(), ApiError> {
        self.send_bucket("HEAD", Vec::new()).map(|_| ())
    }
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
    use std::collections::BTreeMap;

    #[test]
    fn s3_store_checks_bucket_with_signed_head_request() {
        let server = TestS3Server::start(vec![TestS3Response::empty()]);
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
            TestS3Response::empty(),
            TestS3Response::body(b"stored payload"),
            TestS3Response::empty(),
        ]);
        let store = test_s3_store(&server.endpoint);
        let key = "objects/blob-1";

        store.put(key, b"stored payload").unwrap();
        assert_eq!(store.get(key).unwrap(), b"stored payload");
        store.delete(key).unwrap();

        let put = server.recv();
        assert_eq!(put.method, "PUT");
        assert_eq!(put.path, "/scope-bucket/objects/blob-1");
        assert_eq!(put.body, b"stored payload");
        assert_signed_s3_headers(&put);

        let get = server.recv();
        assert_eq!(get.method, "GET");
        assert_eq!(get.path, "/scope-bucket/objects/blob-1");
        assert!(get.body.is_empty());
        assert_signed_s3_headers(&get);

        let delete = server.recv();
        assert_eq!(delete.method, "DELETE");
        assert_eq!(delete.path, "/scope-bucket/objects/blob-1");
        assert!(delete.body.is_empty());
        assert_signed_s3_headers(&delete);
    }

    #[test]
    fn s3_store_bounded_get_rejects_declared_oversized_body_before_reading() {
        let server = TestS3Server::start(vec![TestS3Response::declared_length(5)]);
        let store = test_s3_store(&server.endpoint);

        let error = store.get_bounded("objects/too-large", 4).unwrap_err();

        assert_eq!(error.status, axum::http::StatusCode::PAYLOAD_TOO_LARGE);
        assert!(error.message.contains("exceeds 4 bytes"));
        let request = server.recv();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/scope-bucket/objects/too-large");
        assert_signed_s3_headers(&request);
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

    struct TestS3Response {
        body: Vec<u8>,
        content_length: Option<usize>,
    }

    impl TestS3Response {
        fn empty() -> Self {
            Self {
                body: Vec::new(),
                content_length: None,
            }
        }

        fn body(body: &[u8]) -> Self {
            Self {
                body: body.to_vec(),
                content_length: None,
            }
        }

        fn declared_length(content_length: usize) -> Self {
            Self {
                body: Vec::new(),
                content_length: Some(content_length),
            }
        }
    }

    struct TestS3Server {
        endpoint: String,
        host: String,
        requests: std::sync::mpsc::Receiver<CapturedRequest>,
    }

    impl TestS3Server {
        fn start(responses: Vec<TestS3Response>) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let host = format!("127.0.0.1:{}", addr.port());
            let endpoint = format!("http://{host}");
            let (sender, requests) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                for response in responses {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_request(&mut stream);
                    sender.send(request).unwrap();
                    let content_length = response.content_length.unwrap_or(response.body.len());
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        content_length
                    );
                    use std::io::Write as _;
                    stream.write_all(headers.as_bytes()).unwrap();
                    stream.write_all(&response.body).unwrap();
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
        use std::io::Read as _;

        let mut received = Vec::new();
        let mut buffer = [0_u8; 1024];
        let header_end = loop {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0, "connection closed before headers");
            received.extend_from_slice(&buffer[..count]);
            if let Some(index) = received.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };

        let headers_text = String::from_utf8(received[..header_end].to_vec()).unwrap();
        let mut lines = headers_text.split("\r\n");
        let request_line = lines.next().unwrap();
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().unwrap().to_string();
        let path = request_parts.next().unwrap().to_string();
        let headers = lines
            .filter(|line| !line.is_empty())
            .map(|line| {
                let (name, value) = line.split_once(':').unwrap();
                (name.to_ascii_lowercase(), value.trim().to_string())
            })
            .collect::<BTreeMap<_, _>>();
        let content_length = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = received[header_end..].to_vec();
        while body.len() < content_length {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0, "connection closed before body");
            body.extend_from_slice(&buffer[..count]);
        }
        body.truncate(content_length);

        CapturedRequest {
            method,
            path,
            headers,
            body,
        }
    }
}
