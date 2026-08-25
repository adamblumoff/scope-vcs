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
    AppendAttemptLogRequest, AttemptCacheKeyMaterial, AttemptConclusionRequest,
    AttemptHeartbeatRequest, AttemptHeartbeatResponse, AttemptStatusResponse, ClaimRuntimeResponse,
    CompleteAttemptRequest, CompleteAttemptStepRequest, ReportAttemptCacheFinalizationsRequest,
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

#[derive(Clone)]
pub struct RuntimeClient {
    client: Client,
    api_url: String,
    attempt_id: String,
    attempt_token: Arc<Mutex<Option<String>>>,
    cache_access: Arc<Mutex<Option<CacheAccess>>>,
    cache_keys: Arc<Mutex<Vec<AttemptCacheKeyMaterial>>>,
    heartbeat_lock: Arc<Mutex<()>>,
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
            cache_keys: Arc::new(Mutex::new(Vec::new())),
            heartbeat_lock: Arc::new(Mutex::new(())),
        })
    }

    #[cfg(test)]
    pub(crate) fn disconnected_for_cache_tests() -> Self {
        Self {
            client: Client::new(),
            api_url: "http://127.0.0.1:1".to_string(),
            attempt_id: "test".to_string(),
            attempt_token: Arc::new(Mutex::new(Some("test".to_string()))),
            cache_access: Arc::new(Mutex::new(Some(CacheAccess {
                endpoint: "http://127.0.0.1:1".to_string(),
                grant: "test".to_string(),
            }))),
            cache_keys: Arc::new(Mutex::new(Vec::new())),
            heartbeat_lock: Arc::new(Mutex::new(())),
        }
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
pub(crate) mod cache_client;
mod completion;
pub(crate) mod control;
mod logs;
mod sink;
mod source;

#[cfg(test)]
mod tests;
