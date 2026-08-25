use super::*;

#[derive(Debug)]
pub(crate) enum CacheDownloadError {
    Transport(anyhow::Error),
    Invalid(anyhow::Error),
}

pub(crate) struct CacheDownloadAttempt {
    pub(crate) outcome: Result<(), CacheDownloadError>,
    pub(crate) download_verify_ms: u64,
    pub(crate) sync_ms: u64,
}

impl RuntimeClient {
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
    ) -> CacheDownloadAttempt {
        let download_started = std::time::Instant::now();
        let download = (|| {
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
            Ok(file)
        })();
        let download_verify_ms = crate::cache::types::elapsed_ms(download_started);
        let file = match download {
            Ok(file) => file,
            Err(error) => {
                return CacheDownloadAttempt {
                    outcome: Err(error),
                    download_verify_ms,
                    sync_ms: 0,
                };
            }
        };
        let sync_started = std::time::Instant::now();
        let outcome = file
            .sync_all()
            .context("sync cache archive")
            .map_err(CacheDownloadError::Invalid);
        CacheDownloadAttempt {
            outcome,
            download_verify_ms,
            sync_ms: crate::cache::types::elapsed_ms(sync_started),
        }
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
}

pub(super) fn copy_hashed(
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

pub(super) fn validate_cache_size(size: u64) -> anyhow::Result<()> {
    if size > MAX_CACHE_OBJECT_BYTES {
        bail!("cache exceeds {MAX_CACHE_OBJECT_BYTES} bytes");
    }
    Ok(())
}
