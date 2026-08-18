use crate::settings::RuntimeSettings;
use anyhow::{Context as _, bail};
use reqwest::blocking::{Client, Response};
use scope_api_contract::{
    AppendAttemptLogRequest, AttemptConclusionRequest, AttemptHeartbeatRequest,
    AttemptStatusResponse, CacheDownloadSessionResponse, CacheUploadSessionResponse,
    ClaimRuntimeResponse, CommitCacheUploadRequest, CompleteAttemptRequest,
    CompleteAttemptStepRequest, ReportAttemptCacheFinalizationsRequest,
    ReportAttemptCachePreparationsRequest, StepConclusionRequest,
};
use sha2::{Digest as _, Sha256};
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

#[derive(Clone)]
pub struct RuntimeClient {
    client: Client,
    api_url: String,
    attempt_id: String,
    attempt_token: Arc<Mutex<Option<String>>>,
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
        Ok(response)
    }

    pub fn download_source(&self, expected_digest: &str, destination: &Path) -> anyhow::Result<()> {
        let mut response = self
            .auth(self.client.get(self.url("source")))
            .send()
            .context("download run source")?;
        ensure_success(&response, "download run source")?;
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
            bail!("downloaded source digest does not match attempt");
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

    pub fn append_log(&self, step: u32, sequence: u64, text: String) -> anyhow::Result<()> {
        let _: scope_api_contract::RunLogResponse = self.post_json(
            "logs",
            &AppendAttemptLogRequest {
                step_index: step,
                sequence,
                text,
            },
            "append step log",
        )?;
        Ok(())
    }

    pub fn heartbeat(&self) -> anyhow::Result<AttemptStatusResponse> {
        self.post_json(
            "heartbeat",
            &AttemptHeartbeatRequest {},
            "heartbeat attempt",
        )
    }

    pub fn cache_download_session(
        &self,
        digest: &str,
    ) -> anyhow::Result<CacheDownloadSessionResponse> {
        let response = self
            .auth(self.client.get(self.url(&format!("caches/{digest}"))))
            .send()
            .context("request cache download")?;
        json(response, "request cache download")
    }

    pub fn cache_upload_session(&self, digest: &str) -> anyhow::Result<CacheUploadSessionResponse> {
        self.post_json(
            &format!("caches/{digest}/upload"),
            &serde_json::json!({}),
            "request cache upload",
        )
    }

    pub fn commit_cache(
        &self,
        digest: &str,
        request: &CommitCacheUploadRequest,
    ) -> anyhow::Result<()> {
        self.post_empty(
            &format!("caches/{digest}/commit"),
            request,
            "commit cache upload",
        )
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
    ) -> anyhow::Result<()> {
        if expected_size > 10 * 1024 * 1024 * 1024 {
            bail!("cache exceeds 10 GiB");
        }
        let mut response = self
            .client
            .get(url)
            .send()
            .context("download cache object")?;
        ensure_success(&response, "download cache object")?;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)
            .context("create cache archive")?;
        copy_hashed(&mut response, &mut file, expected_size, expected_checksum)?;
        file.sync_all().context("sync cache archive")
    }

    pub fn upload_cache(&self, url: &str, source: &Path) -> anyhow::Result<()> {
        let file = fs::File::open(source).context("open cache archive")?;
        let size = file.metadata()?.len();
        let response = self
            .client
            .put(url)
            .header(reqwest::header::CONTENT_LENGTH, size)
            .body(file)
            .send()
            .context("upload cache object")?;
        ensure_success(&response, "upload cache object")
    }

    pub fn complete_step(
        &self,
        step: u32,
        exit_code: i32,
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
                logs_truncated: false,
            },
            "complete step",
        )
    }

    pub fn complete_timeout(&self) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::TimedOut)
    }

    pub fn complete_succeeded(&self) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::Succeeded)
    }

    pub fn complete_canceled(&self) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::Canceled)
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
        self.complete(AttemptConclusionRequest::SetupFailed {
            exit_code: 70,
            message: message.chars().take(2048).collect(),
        })
    }

    fn complete(&self, conclusion: AttemptConclusionRequest) -> anyhow::Result<()> {
        let _: AttemptStatusResponse = self.post_json(
            "complete",
            &CompleteAttemptRequest {
                conclusion,
                logs_truncated: false,
            },
            "complete attempt",
        )?;
        Ok(())
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
) -> anyhow::Result<()> {
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .context("cache size overflow")?;
        if total > expected_size {
            bail!("cache object exceeds declared size");
        }
        writer.write_all(&buffer[..read])?;
        hasher.update(&buffer[..read]);
    }
    if total != expected_size || hex::encode(hasher.finalize()) != expected_checksum {
        bail!("cache object integrity check failed");
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
