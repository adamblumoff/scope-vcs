use anyhow::Context;
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub(super) struct RunnerConfig {
    pub(super) api_url: String,
    pub(super) runner_id: String,
    pub(super) name: String,
    pub(super) secret: String,
    pub(super) cache_root: Option<PathBuf>,
}

pub(super) fn configured_cache_root() -> Option<PathBuf> {
    env::var_os("SCOPE_RUNNER_CACHE_ROOT")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

pub(super) fn runner_config_path() -> anyhow::Result<PathBuf> {
    if let Some(path) = env::var_os("SCOPE_RUNNER_CONFIG").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    Ok(scope_config_home()?.join("scope/runner.json"))
}

pub(super) fn store_runner_config(path: &Path, config: &RunnerConfig) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .context("runner config path has no parent directory")?;
    fs::create_dir_all(parent).context("create runner config directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut permissions = fs::metadata(parent)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(parent, permissions)?;
        let temp = path.with_extension(format!("tmp.{}", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temp)
            .context("create runner config")?;
        serde_json::to_writer(&mut file, config).context("serialize runner config")?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(temp, path).context("install runner config")?;
    }
    #[cfg(not(unix))]
    {
        fs::write(path, serde_json::to_vec(config)?).context("write runner config")?;
    }
    Ok(())
}

pub(super) fn load_runner_config() -> anyhow::Result<RunnerConfig> {
    load_runner_config_from(&runner_config_path()?)
}

pub(super) fn load_runner_config_from(path: &Path) -> anyhow::Result<RunnerConfig> {
    let bytes = fs::read(path).with_context(|| {
        format!(
            "read runner config {}; run scope runner install",
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes).context("parse runner config")
}

pub(super) fn scope_config_home() -> anyhow::Result<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|path| !path.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .context("XDG_CONFIG_HOME or HOME is required")
}
