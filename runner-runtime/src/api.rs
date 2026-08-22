use crate::{
    execute::{AppendLogError, AppendLogOutcome, ExecutionSink},
    settings::RuntimeSettings,
};
use anyhow::{Context as _, bail};
use reqwest::{
    StatusCode,
    blocking::{Client, Response},
};
use scope_api_contract::{
    AppendAttemptLogRequest, AttemptConclusionRequest, AttemptHeartbeatRequest,
    AttemptHeartbeatResponse, AttemptStatusResponse, ClaimRuntimeResponse, CompleteAttemptRequest,
    CompleteAttemptStepRequest, ReportAttemptCacheFinalizationsRequest,
    ReportAttemptCachePreparationsRequest, StepConclusionRequest,
};
use scope_cache_contract::{
    COMMIT_CACHE_UPLOAD_PATH, CommitCacheUploadRequest, CommitCacheUploadResponse,
    PREPARE_CACHE_UPLOAD_PATH, PrepareCacheUploadRequest, PrepareCacheUploadResponse,
    RESTORE_CACHE_PATH, RestoreCacheRequest, RestoreCacheResponse,
};
use scope_cache_domain::MAX_CACHE_OBJECT_BYTES;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const LOG_APPEND_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(crate) enum CacheDownloadError {
    Transport(anyhow::Error),
    Invalid(anyhow::Error),
}

#[derive(Clone)]
pub struct RuntimeClient {
    client: Client,
    api_url: String,
    attempt_id: String,
    attempt_token: Arc<Mutex<Option<String>>>,
    cache_access: Arc<Mutex<Option<CacheAccess>>>,
}

#[derive(Clone)]
struct CacheAccess {
    endpoint: String,
    grant: String,
}

impl RuntimeClient {
    pub fn new(settings: &RuntimeSettings) -> anyhow::Result<Self> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(20))
                .timeout(Duration::from_secs(15 * 60))
                .build()
                .context("build Scope API client")?,
            api_url: settings.api_url.clone(),
            attempt_id: settings.attempt_id.clone(),
            attempt_token: Arc::new(Mutex::new(None)),
            cache_access: Arc::new(Mutex::new(None)),
        })
    }

    pub fn claim(&self, bootstrap_token: &str) -> anyhow::Result<ClaimRuntimeResponse> {
        let response = self
            .client
            .post(self.url("claim"))
            .bearer_auth(bootstrap_token)
            .send()
            .context("claim cloud run attempt")?;
        let response: ClaimRuntimeResponse = json(response, "claim cloud run attempt")?;
        *self
            .attempt_token
            .lock()
            .expect("attempt token mutex poisoned") = Some(response.attempt_token.clone());
        *self
            .cache_access
            .lock()
            .expect("cache access mutex poisoned") = Some(CacheAccess {
            endpoint: response.cache_endpoint.clone(),
            grant: response.cache_grant.clone(),
        });
        Ok(response)
    }

    pub fn download_source(
        &self,
        expected_identity: &str,
        destination: &Path,
    ) -> anyhow::Result<()> {
        let mut response = self
            .auth(self.client.get(self.url("source")))
            .send()
            .context("download run source")?;
        ensure_success(&response, "download run source")?;
        let source_identity = response
            .headers()
            .get("x-scope-source-identity")
            .and_then(|value| value.to_str().ok())
            .context("run source identity header is missing or invalid")?;
        if source_identity != expected_identity {
            bail!("downloaded source identity does not match attempt");
        }
        let expected_digest = response
            .headers()
            .get("x-scope-source-sha256")
            .and_then(|value| value.to_str().ok())
            .context("run source digest header is missing or invalid")?;
        if expected_digest.len() != 64
            || !expected_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("run source digest header is invalid");
        }
        let expected_digest = expected_digest.to_string();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_SOURCE_BYTES)
        {
            bail!("run source exceeds {MAX_SOURCE_BYTES} bytes");
        }
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)
            .context("create source bundle")?;
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = response.read(&mut buffer).context("read source bundle")?;
            if read == 0 {
                break;
            }
            total = total
                .checked_add(read as u64)
                .context("source size overflow")?;
            if total > MAX_SOURCE_BYTES {
                bail!("run source exceeds {MAX_SOURCE_BYTES} bytes");
            }
            file.write_all(&buffer[..read])
                .context("write source bundle")?;
            hasher.update(&buffer[..read]);
        }
        file.sync_all().context("sync source bundle")?;
        if hex::encode(hasher.finalize()) != expected_digest {
            bail!("downloaded source bytes do not match response digest");
        }
        Ok(())
    }

    pub fn start_step(&self, step: u32) -> anyhow::Result<AttemptStatusResponse> {
        self.post_json(
            &format!("steps/{step}/start"),
            &serde_json::json!({}),
            "start step",
        )
    }

    pub fn append_log(
        &self,
        step: u32,
        sequence: u64,
        text: &str,
    ) -> Result<AppendLogOutcome, AppendLogError> {
        let response = self
            .auth(self.client.post(self.url("logs")))
            .timeout(LOG_APPEND_TIMEOUT)
            .json(&AppendAttemptLogRequest {
                step_index: step,
                sequence,
                text: text.to_owned(),
            })
            .send()
            .context("append step log")
            .map_err(AppendLogError::retryable)?;
        let status = response.status();
        if status.is_success() {
            return Ok(AppendLogOutcome::Accepted);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Ok(AppendLogOutcome::Truncated);
        }
        let error = anyhow::anyhow!("append step log: Scope API returned {status}");
        if status == StatusCode::REQUEST_TIMEOUT || status.is_server_error() {
            Err(AppendLogError::retryable(error))
        } else {
            Err(AppendLogError::fatal(error))
        }
    }

    pub fn heartbeat(&self) -> anyhow::Result<AttemptStatusResponse> {
        let response: AttemptHeartbeatResponse = self.post_json(
            "heartbeat",
            &AttemptHeartbeatRequest {},
            "heartbeat attempt",
        )?;
        let mut access = self
            .cache_access
            .lock()
            .expect("cache access mutex poisoned");
        let access = access
            .as_mut()
            .context("cache access is unavailable before attempt claim")?;
        access.grant = response.cache_grant;
        Ok(response.status)
    }

    pub fn restore_cache(
        &self,
        request: &RestoreCacheRequest,
    ) -> anyhow::Result<RestoreCacheResponse> {
        self.cache_post(RESTORE_CACHE_PATH, request, "restore cache")
    }

    pub fn prepare_cache_upload(
        &self,
        request: &PrepareCacheUploadRequest,
    ) -> anyhow::Result<PrepareCacheUploadResponse> {
        self.cache_post(PREPARE_CACHE_UPLOAD_PATH, request, "prepare cache upload")
    }

    pub fn commit_cache_upload(
        &self,
        request: &CommitCacheUploadRequest,
    ) -> anyhow::Result<CommitCacheUploadResponse> {
        self.cache_post(COMMIT_CACHE_UPLOAD_PATH, request, "commit cache upload")
    }

    pub fn report_cache_preparations(
        &self,
        request: &ReportAttemptCachePreparationsRequest,
    ) -> anyhow::Result<()> {
        self.post_empty(
            "cache-observations/preparations",
            request,
            "report cache preparations",
        )
    }

    pub fn report_cache_finalizations(
        &self,
        request: &ReportAttemptCacheFinalizationsRequest,
    ) -> anyhow::Result<()> {
        self.post_empty(
            "cache-observations/finalizations",
            request,
            "report cache finalizations",
        )
    }

    pub fn download_cache(
        &self,
        url: &str,
        destination: &Path,
        expected_size: u64,
        expected_checksum: &str,
    ) -> Result<(), CacheDownloadError> {
        validate_cache_size(expected_size).map_err(CacheDownloadError::Invalid)?;
        let mut response = self
            .client
            .get(url)
            .send()
            .context("download cache object")
            .map_err(CacheDownloadError::Transport)?;
        ensure_success(&response, "download cache object")
            .map_err(CacheDownloadError::Transport)?;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)
            .context("create cache archive")
            .map_err(CacheDownloadError::Invalid)?;
        copy_hashed(&mut response, &mut file, expected_size, expected_checksum)?;
        file.sync_all()
            .context("sync cache archive")
            .map_err(CacheDownloadError::Invalid)
    }

    pub fn upload_cache(
        &self,
        url: &str,
        headers: &BTreeMap<String, String>,
        source: &Path,
    ) -> anyhow::Result<()> {
        let file = fs::File::open(source).context("open cache archive")?;
        let size = file.metadata()?.len();
        let declared_size = headers
            .get("content-length")
            .context("cache upload instructions are missing content-length")?
            .parse::<u64>()
            .context("cache upload content-length is invalid")?;
        if declared_size != size {
            bail!("cache archive size does not match signed upload size");
        }
        let mut request = self.client.put(url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request.body(file).send().context("upload cache object")?;
        ensure_success(&response, "upload cache object")
    }

    fn cache_post<T: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        request: &T,
        context: &'static str,
    ) -> anyhow::Result<R> {
        let access = self
            .cache_access
            .lock()
            .expect("cache access mutex poisoned")
            .clone()
            .context("cache access is unavailable before attempt claim")?;
        let response = self
            .client
            .post(format!("{}{}", access.endpoint, path))
            .bearer_auth(access.grant)
            .json(request)
            .send()
            .with_context(|| context.to_string())?;
        json(response, context)
    }

    pub fn complete_step(
        &self,
        step: u32,
        exit_code: i32,
        logs_truncated: bool,
    ) -> anyhow::Result<AttemptStatusResponse> {
        let conclusion = if exit_code == 0 {
            StepConclusionRequest::Succeeded
        } else {
            StepConclusionRequest::Failed { exit_code }
        };
        self.post_json(
            &format!("steps/{step}/complete"),
            &CompleteAttemptStepRequest {
                conclusion,
                logs_truncated,
            },
            "complete step",
        )
    }

    pub fn complete_timeout(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::TimedOut, logs_truncated)
    }

    pub fn complete_succeeded(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::Succeeded, logs_truncated)
    }

    pub fn complete_canceled(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::Canceled, logs_truncated)
    }

    pub fn complete_setup_failure(&self, message: &str) -> anyhow::Result<()> {
        if self
            .attempt_token
            .lock()
            .expect("attempt token mutex poisoned")
            .is_none()
        {
            return Ok(());
        }
        self.complete(
            AttemptConclusionRequest::SetupFailed {
                exit_code: 70,
                message: message.chars().take(2048).collect(),
            },
            false,
        )
    }

    fn complete(
        &self,
        conclusion: AttemptConclusionRequest,
        logs_truncated: bool,
    ) -> anyhow::Result<()> {
        let _: AttemptStatusResponse = self.post_json(
            "complete",
            &CompleteAttemptRequest {
                conclusion,
                logs_truncated,
            },
            "complete attempt",
        )?;
        Ok(())
    }

    pub fn abandon(&self) -> anyhow::Result<()> {
        let response = self
            .auth(self.client.post(self.url("abandon")))
            .send()
            .context("abandon attempt")?;
        ensure_success(&response, "abandon attempt")
    }

    fn post_json<T: serde::de::DeserializeOwned>(
        &self,
        action: &str,
        body: &impl serde::Serialize,
        label: &str,
    ) -> anyhow::Result<T> {
        let response = self
            .auth(self.client.post(self.url(action)))
            .json(body)
            .send()
            .with_context(|| label.to_string())?;
        json(response, label)
    }

    fn post_empty(
        &self,
        action: &str,
        body: &impl serde::Serialize,
        label: &str,
    ) -> anyhow::Result<()> {
        let response = self
            .auth(self.client.post(self.url(action)))
            .json(body)
            .send()
            .with_context(|| label.to_string())?;
        ensure_success(&response, label)?;
        Ok(())
    }

    fn auth(
        &self,
        request: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        let token = self
            .attempt_token
            .lock()
            .expect("attempt token mutex poisoned")
            .clone()
            .expect("runtime must claim before attempt operations");
        request.bearer_auth(token)
    }

    fn url(&self, action: &str) -> String {
        format!(
            "{}/v1/runtime-protocol/attempts/{}/{}",
            self.api_url, self.attempt_id, action
        )
    }
}

impl ExecutionSink for RuntimeClient {
    fn start_step(&self, step: u32) -> anyhow::Result<bool> {
        Ok(RuntimeClient::start_step(self, step)?.cancellation_requested)
    }

    fn append_log(
        &self,
        step: u32,
        sequence: u64,
        text: &str,
    ) -> Result<AppendLogOutcome, AppendLogError> {
        RuntimeClient::append_log(self, step, sequence, text)
    }

    fn heartbeat(&self) -> anyhow::Result<bool> {
        Ok(RuntimeClient::heartbeat(self)?.cancellation_requested)
    }

    fn complete_step(&self, step: u32, exit_code: i32, logs_truncated: bool) -> anyhow::Result<()> {
        RuntimeClient::complete_step(self, step, exit_code, logs_truncated)?;
        Ok(())
    }

    fn complete_timeout(&self, logs_truncated: bool) -> anyhow::Result<()> {
        RuntimeClient::complete_timeout(self, logs_truncated)
    }

    fn complete_canceled(&self, logs_truncated: bool) -> anyhow::Result<()> {
        RuntimeClient::complete_canceled(self, logs_truncated)
    }

    fn abandon(&self) -> anyhow::Result<()> {
        RuntimeClient::abandon(self)
    }
}

pub struct RuntimeHeartbeat {
    stop: mpsc::Sender<()>,
    canceled: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl RuntimeHeartbeat {
    pub fn start(client: RuntimeClient) -> Self {
        let (stop, receiver) = mpsc::channel();
        let canceled = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let canceled_in_thread = Arc::clone(&canceled);
        let failed_in_thread = Arc::clone(&failed);
        let thread = thread::spawn(move || {
            loop {
                match receiver.recv_timeout(Duration::from_secs(10)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => match client.heartbeat() {
                        Ok(status) if status.cancellation_requested => {
                            canceled_in_thread.store(true, Ordering::Release);
                            break;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            eprintln!("runtime heartbeat failed: {error:#}");
                            failed_in_thread.store(true, Ordering::Release);
                            break;
                        }
                    },
                }
            }
        });
        Self {
            stop,
            canceled,
            failed,
            thread: Some(thread),
        }
    }

    pub fn finish(mut self) -> anyhow::Result<bool> {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("runtime heartbeat thread panicked"))?;
        }
        if self.failed.load(Ordering::Acquire) {
            bail!("runtime lost contact with the Scope API");
        }
        Ok(self.canceled.load(Ordering::Acquire))
    }
}

impl Drop for RuntimeHeartbeat {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn copy_hashed(
    reader: &mut impl Read,
    writer: &mut impl Write,
    expected_size: u64,
    expected_checksum: &str,
) -> Result<(), CacheDownloadError> {
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .context("read cache object")
            .map_err(CacheDownloadError::Transport)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .context("cache size overflow")
            .map_err(CacheDownloadError::Invalid)?;
        if total > expected_size {
            return Err(CacheDownloadError::Invalid(anyhow::anyhow!(
                "cache object exceeds declared size"
            )));
        }
        writer
            .write_all(&buffer[..read])
            .context("write cache object")
            .map_err(CacheDownloadError::Invalid)?;
        hasher.update(&buffer[..read]);
    }
    if total != expected_size || hex::encode(hasher.finalize()) != expected_checksum {
        return Err(CacheDownloadError::Invalid(anyhow::anyhow!(
            "cache object integrity check failed"
        )));
    }
    Ok(())
}

fn validate_cache_size(size: u64) -> anyhow::Result<()> {
    if size > MAX_CACHE_OBJECT_BYTES {
        bail!("cache exceeds {MAX_CACHE_OBJECT_BYTES} bytes");
    }
    Ok(())
}

fn json<T: serde::de::DeserializeOwned>(response: Response, label: &str) -> anyhow::Result<T> {
    ensure_success(&response, label)?;
    response
        .json()
        .with_context(|| format!("decode {label} response"))
}

fn ensure_success(response: &Response, label: &str) -> anyhow::Result<()> {
    if !response.status().is_success() {
        bail!("{label}: Scope API returned {}", response.status());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, sync::mpsc};

    #[test]
    fn cache_download_limit_is_one_gibibyte() {
        assert!(validate_cache_size(MAX_CACHE_OBJECT_BYTES).is_ok());
        let error = validate_cache_size(MAX_CACHE_OBJECT_BYTES + 1).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("cache exceeds {MAX_CACHE_OBJECT_BYTES} bytes")
        );
    }

    #[test]
    fn cache_integrity_failures_are_invalid_not_transport_errors() {
        let mut source = &b"different"[..];
        let mut destination = Vec::new();
        let error = copy_hashed(&mut source, &mut destination, 9, "wrong").unwrap_err();
        assert!(matches!(error, CacheDownloadError::Invalid(_)));
    }

    #[test]
    fn append_log_treats_only_rate_limiting_as_truncation() {
        let (client, requests, server) = test_client(&[
            ("429 Too Many Requests", ""),
            ("408 Request Timeout", ""),
            ("500 Internal Server Error", ""),
            ("409 Conflict", ""),
        ]);

        assert_eq!(
            client.append_log(2, 7, "first").unwrap(),
            AppendLogOutcome::Truncated
        );
        assert!(matches!(
            client.append_log(2, 7, "first"),
            Err(AppendLogError::Retryable(_))
        ));
        assert!(matches!(
            client.append_log(2, 7, "first"),
            Err(AppendLogError::Retryable(_))
        ));
        assert!(matches!(
            client.append_log(2, 7, "first"),
            Err(AppendLogError::Fatal(_))
        ));
        for _ in 0..4 {
            requests.recv_timeout(Duration::from_secs(1)).unwrap();
        }
        server.join().unwrap();
    }

    #[test]
    fn completion_requests_report_the_accumulated_truncation() {
        let status =
            r#"{"state":"succeeded","cancellation_requested":false,"lease_expires_at_unix":0}"#;
        let (client, requests, server) = test_client(&[("200 OK", status), ("200 OK", status)]);

        client.complete_step(3, 0, true).unwrap();
        client.complete_succeeded(true).unwrap();

        let step_request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
        let attempt_request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            step_request.starts_with("POST /v1/runtime-protocol/attempts/test/steps/3/complete")
        );
        assert!(attempt_request.starts_with("POST /v1/runtime-protocol/attempts/test/complete"));
        assert_eq!(request_json(&step_request)["logs_truncated"], true);
        assert_eq!(request_json(&attempt_request)["logs_truncated"], true);
        server.join().unwrap();
    }

    fn test_client(
        responses: &[(&'static str, &'static str)],
    ) -> (
        RuntimeClient,
        mpsc::Receiver<String>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let responses = responses.to_vec();
        let (request_sender, requests) = mpsc::channel();
        let server = thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                request_sender.send(request).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let client = RuntimeClient {
            client: Client::builder()
                .timeout(Duration::from_secs(1))
                .build()
                .unwrap(),
            api_url: format!("http://{address}"),
            attempt_id: "test".to_string(),
            attempt_token: Arc::new(Mutex::new(Some("token".to_string()))),
            cache_access: Arc::new(Mutex::new(None)),
        };
        (client, requests, server)
    }

    fn read_request(stream: &mut std::net::TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            request.extend_from_slice(&buffer[..read]);
            let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
                continue;
            };
            let header_end = header_end + 4;
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .and_then(|length| length.parse::<usize>().ok())
                .unwrap_or(0);
            if request.len() >= header_end + content_length {
                return String::from_utf8(request).unwrap();
            }
        }
    }

    fn request_json(request: &str) -> serde_json::Value {
        let (_, body) = request.split_once("\r\n\r\n").unwrap();
        serde_json::from_str(body).unwrap()
    }
}
