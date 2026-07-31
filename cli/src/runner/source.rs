use crate::api::attempt_source;
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    time::Duration,
};

const MAX_SOURCE_BUNDLE_BYTES: u64 = 128 * 1024 * 1024;

pub(super) fn download_attempt_source(
    client: &Client,
    api_url: &str,
    attempt_token: &str,
    attempt_id: &str,
    expected_digest: &str,
    destination: &Path,
) -> anyhow::Result<()> {
    let mut response = attempt_source(client, api_url, attempt_token, attempt_id)?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_SOURCE_BUNDLE_BYTES)
    {
        bail!("run source bundle exceeds {MAX_SOURCE_BUNDLE_BYTES} bytes");
    }
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .context("create run source bundle")?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .context("stream run source bundle")?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .context("run source bundle byte count overflow")?;
        if total > MAX_SOURCE_BUNDLE_BYTES {
            bail!("run source bundle exceeds {MAX_SOURCE_BUNDLE_BYTES} bytes");
        }
        file.write_all(&buffer[..read])
            .context("write run source bundle")?;
        hasher.update(&buffer[..read]);
    }
    file.sync_all().context("sync run source bundle")?;
    let actual_digest = format!("{:x}", hasher.finalize());
    if actual_digest != expected_digest {
        bail!("downloaded source digest does not match claimed job");
    }
    Ok(())
}

pub(super) fn source_download_client() -> anyhow::Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(15 * 60))
        .build()
        .context("build run source download client")
}
