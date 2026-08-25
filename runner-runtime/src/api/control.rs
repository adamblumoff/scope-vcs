use super::*;

impl RuntimeClient {
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

    pub fn start_step(&self, step: u32) -> anyhow::Result<AttemptStatusResponse> {
        self.post_json(
            &format!("steps/{step}/start"),
            &serde_json::json!({}),
            "start step",
        )
    }

    pub fn heartbeat(&self) -> anyhow::Result<AttemptStatusResponse> {
        let _heartbeat = self
            .heartbeat_lock
            .lock()
            .expect("heartbeat mutex poisoned");
        let cache_keys = self
            .cache_keys
            .lock()
            .expect("cache keys mutex poisoned")
            .clone();
        let response: AttemptHeartbeatResponse = self.post_json(
            "heartbeat",
            &AttemptHeartbeatRequest { cache_keys },
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

    pub fn authorize_cache_keys(
        &self,
        cache_keys: Vec<AttemptCacheKeyMaterial>,
    ) -> anyhow::Result<()> {
        *self.cache_keys.lock().expect("cache keys mutex poisoned") = cache_keys;
        self.heartbeat()?;
        Ok(())
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
