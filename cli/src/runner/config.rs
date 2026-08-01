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

pub(super) fn runner_cache_root(existing: Option<&Path>) -> anyhow::Result<PathBuf> {
    cache_root_from(
        env::var_os("SCOPE_RUNNER_CACHE_ROOT"),
        existing.map(Path::to_path_buf),
        env::var_os("XDG_CACHE_HOME"),
        env::var_os("HOME"),
    )
}

pub(super) fn runner_cache_root_is_disposable_default(existing: Option<&Path>) -> bool {
    cache_root_is_disposable_default(
        env::var_os("SCOPE_RUNNER_CACHE_ROOT"),
        existing,
        env::var_os("XDG_CACHE_HOME"),
        env::var_os("HOME"),
    )
}

fn cache_root_from(
    configured: Option<std::ffi::OsString>,
    existing: Option<PathBuf>,
    cache_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> anyhow::Result<PathBuf> {
    if let Some(path) = configured
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
    {
        return Ok(path);
    }
    if let Some(path) = existing {
        return Ok(path);
    }
    default_cache_root(cache_home, home).context("XDG_CACHE_HOME or HOME is required")
}

fn default_cache_root(
    cache_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    cache_home
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            home.filter(|path| !path.is_empty())
                .map(|home| PathBuf::from(home).join(".cache"))
        })
        .map(|root| root.join("scope/runner"))
}

fn cache_root_is_disposable_default(
    configured: Option<std::ffi::OsString>,
    existing: Option<&Path>,
    cache_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> bool {
    if configured.is_some_and(|path| !path.is_empty()) {
        return false;
    }
    let Some(default) = default_cache_root(cache_home, home) else {
        return false;
    };
    existing.is_none_or(|path| path == default)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn cache_root_prefers_the_explicit_runner_override() {
        assert_eq!(
            cache_root_from(
                Some(OsString::from("/mnt/scope-cache")),
                Some(PathBuf::from("/srv/existing-cache")),
                Some(OsString::from("/var/cache/user")),
                Some(OsString::from("/home/runner")),
            )
            .unwrap(),
            PathBuf::from("/mnt/scope-cache")
        );
    }

    #[test]
    fn cache_root_uses_xdg_then_the_home_cache_directory() {
        assert_eq!(
            cache_root_from(
                None,
                None,
                Some(OsString::from("/var/cache/user")),
                Some(OsString::from("/home/runner")),
            )
            .unwrap(),
            PathBuf::from("/var/cache/user/scope/runner")
        );
        assert_eq!(
            cache_root_from(None, None, None, Some(OsString::from("/home/runner")),).unwrap(),
            PathBuf::from("/home/runner/.cache/scope/runner")
        );
    }

    #[test]
    fn cache_root_preserves_an_installed_path_without_an_override() {
        assert_eq!(
            cache_root_from(
                None,
                Some(PathBuf::from("/srv/scope-cache")),
                Some(OsString::from("/var/cache/user")),
                Some(OsString::from("/home/runner")),
            )
            .unwrap(),
            PathBuf::from("/srv/scope-cache")
        );
    }

    #[test]
    fn only_the_implicit_default_cache_is_disposable() {
        let cache_home = Some(OsString::from("/var/cache/user"));
        let home = Some(OsString::from("/home/runner"));
        assert!(cache_root_is_disposable_default(
            None,
            Some(Path::new("/var/cache/user/scope/runner")),
            cache_home.clone(),
            home.clone(),
        ));
        assert!(cache_root_is_disposable_default(
            None,
            None,
            cache_home.clone(),
            home.clone(),
        ));
        assert!(!cache_root_is_disposable_default(
            None,
            Some(Path::new("/srv/scope-cache")),
            cache_home.clone(),
            home.clone(),
        ));
        assert!(!cache_root_is_disposable_default(
            Some(OsString::from("/var/cache/user/scope/runner")),
            Some(Path::new("/var/cache/user/scope/runner")),
            cache_home,
            home,
        ));
    }
}
