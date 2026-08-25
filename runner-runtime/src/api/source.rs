use super::*;

const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;

impl RuntimeClient {
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
}
