use std::path::Path;

fn unsupported() -> anyhow::Result<()> {
    anyhow::bail!("self-hosted Scope runners require Linux")
}

pub fn install(_name: &str, _repository: &str) -> anyhow::Result<()> {
    unsupported()
}

pub fn status() -> anyhow::Result<()> {
    unsupported()
}

pub fn add_repository(_repository: &str) -> anyhow::Result<()> {
    unsupported()
}

pub fn remove_repository(_repository: &str) -> anyhow::Result<()> {
    unsupported()
}

pub fn doctor() -> anyhow::Result<()> {
    unsupported()
}

pub fn list_caches() -> anyhow::Result<()> {
    unsupported()
}

pub fn prune_caches(_all: bool) -> anyhow::Result<()> {
    unsupported()
}

pub fn daemon(_config_path: Option<&Path>) -> anyhow::Result<()> {
    unsupported()
}
