use anyhow::{Context as _, bail};
use std::{env, path::PathBuf};

pub struct RuntimeSettings {
    pub api_url: String,
    pub attempt_id: String,
    pub bootstrap_token: String,
    pub work_root: PathBuf,
}

impl RuntimeSettings {
    pub fn from_env() -> anyhow::Result<Self> {
        let api_url = required("SCOPE_API_URL")?.trim_end_matches('/').to_string();
        if !(api_url.starts_with("https://") || api_url.starts_with("http://127.0.0.1")) {
            bail!("SCOPE_API_URL must use HTTPS outside local development");
        }
        Ok(Self {
            api_url,
            attempt_id: required("SCOPE_ATTEMPT_ID")?,
            bootstrap_token: required("SCOPE_BOOTSTRAP_TOKEN")?,
            work_root: env::var_os("SCOPE_WORK_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/scope/work")),
        })
    }

    pub fn prepare_work_directory(&self) -> anyhow::Result<PathBuf> {
        let root = self.work_root.join(&self.attempt_id);
        if root.exists() {
            bail!("attempt work directory already exists");
        }
        std::fs::create_dir_all(&root).context("create attempt work directory")?;
        Ok(root)
    }
}

fn required(name: &str) -> anyhow::Result<String> {
    env::var_os(name)
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("{name} is required"))
}
