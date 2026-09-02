use super::*;

pub(super) const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SOURCE_DOWNLOAD_ATTEMPTS: usize = 3;
const SOURCE_RETRY_DELAY: Duration = Duration::from_millis(100);
const SOURCE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

enum SourceDownloadError {
    Retryable(anyhow::Error),
    Fatal(anyhow::Error),
}

impl RuntimeClient {
    pub fn download_source(
        &self,
        expected_identity: &str,
        destination: &Path,
    ) -> anyhow::Result<()> {
        let started_at = std::time::Instant::now();
        for attempt in 1..=MAX_SOURCE_DOWNLOAD_ATTEMPTS {
            let remaining = SOURCE_DOWNLOAD_TIMEOUT
                .checked_sub(started_at.elapsed())
                .context("download run source exceeded its total timeout")?;
            match self.download_source_once(expected_identity, destination, remaining) {
                Ok(()) => return Ok(()),
                Err(SourceDownloadError::Fatal(error)) => return Err(error),
                Err(SourceDownloadError::Retryable(error))
                    if attempt < MAX_SOURCE_DOWNLOAD_ATTEMPTS =>
                {
                    let delay = source_retry_delay(attempt);
                    if started_at.elapsed().saturating_add(delay) >= SOURCE_DOWNLOAD_TIMEOUT {
                        return Err(error.context("download run source exceeded its total timeout"));
                    }
                    eprintln!("download run source attempt {attempt} failed: {error:#}; retrying");
                    thread::sleep(delay);
                }
                Err(SourceDownloadError::Retryable(error)) => {
                    return Err(error.context(format!(
                        "download run source failed after {MAX_SOURCE_DOWNLOAD_ATTEMPTS} attempts"
                    )));
                }
            }
        }
        unreachable!("source download attempt loop must return")
    }

    fn download_source_once(
        &self,
        expected_identity: &str,
        destination: &Path,
        timeout: Duration,
    ) -> Result<(), SourceDownloadError> {
        let mut response = self
            .auth(self.client.get(self.url("source")).timeout(timeout))
            .send()
            .map_err(|error| {
                SourceDownloadError::Retryable(
                    anyhow::Error::new(error).context("download run source"),
                )
            })?;
        if !response.status().is_success() {
            let status = response.status();
            let error = anyhow::anyhow!("download run source: Scope API returned {status}");
            return Err(if retryable_source_status(status) {
                SourceDownloadError::Retryable(error)
            } else {
                SourceDownloadError::Fatal(error)
            });
        }

        let source_identity = response
            .headers()
            .get("x-scope-source-identity")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                SourceDownloadError::Fatal(anyhow::anyhow!(
                    "run source identity header is missing or invalid"
                ))
            })?;
        if source_identity != expected_identity {
            return Err(SourceDownloadError::Fatal(anyhow::anyhow!(
                "downloaded source identity does not match attempt"
            )));
        }
        let expected_digest = response
            .headers()
            .get("x-scope-source-sha256")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                SourceDownloadError::Fatal(anyhow::anyhow!(
                    "run source digest header is missing or invalid"
                ))
            })?;
        if expected_digest.len() != 64
            || !expected_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(SourceDownloadError::Fatal(anyhow::anyhow!(
                "run source digest header is invalid"
            )));
        }
        let expected_digest = expected_digest.to_string();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_SOURCE_BYTES)
        {
            return Err(SourceDownloadError::Fatal(anyhow::anyhow!(
                "run source exceeds {MAX_SOURCE_BYTES} bytes"
            )));
        }

        let parent = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
            SourceDownloadError::Fatal(anyhow::Error::new(error).context("create source bundle"))
        })?;
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = response.read(&mut buffer).map_err(|error| {
                SourceDownloadError::Retryable(
                    anyhow::Error::new(error).context("read source bundle"),
                )
            })?;
            if read == 0 {
                break;
            }
            total = total.checked_add(read as u64).ok_or_else(|| {
                SourceDownloadError::Fatal(anyhow::anyhow!("source size overflow"))
            })?;
            if total > MAX_SOURCE_BYTES {
                return Err(SourceDownloadError::Fatal(anyhow::anyhow!(
                    "run source exceeds {MAX_SOURCE_BYTES} bytes"
                )));
            }
            staged.write_all(&buffer[..read]).map_err(|error| {
                SourceDownloadError::Fatal(anyhow::Error::new(error).context("write source bundle"))
            })?;
            hasher.update(&buffer[..read]);
        }
        staged.as_file().sync_all().map_err(|error| {
            SourceDownloadError::Fatal(anyhow::Error::new(error).context("sync source bundle"))
        })?;
        if hex::encode(hasher.finalize()) != expected_digest {
            return Err(SourceDownloadError::Fatal(anyhow::anyhow!(
                "downloaded source bytes do not match response digest"
            )));
        }
        staged.persist_noclobber(destination).map_err(|error| {
            SourceDownloadError::Fatal(
                anyhow::Error::new(error.error).context("install source bundle"),
            )
        })?;
        Ok(())
    }
}

fn source_retry_delay(failed_attempt: usize) -> Duration {
    let base = SOURCE_RETRY_DELAY * failed_attempt as u32;
    let mut random = [0_u8; 1];
    let jitter = if getrandom::fill(&mut random).is_ok() {
        base.mul_f64(f64::from(random[0]) / 510.0)
    } else {
        Duration::ZERO
    };
    base + jitter
}

pub(super) fn retryable_source_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}
