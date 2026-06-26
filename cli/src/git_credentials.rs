use anyhow::{Context, bail};
use reqwest::Url;
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const GIT_CREDENTIAL_USERNAME: &str = "scope";
const GIT_PROACTIVE_AUTH_CONFIG: &str = "http.proactiveAuth=basic";

#[derive(Debug, PartialEq, Eq)]
pub struct GitClonePlan {
    pub credential_remote_url: String,
    pub credential_fields: Vec<String>,
    pub credential_store_path: PathBuf,
    pub helper_config_key: String,
    pub helper_config_value: String,
    pub use_http_path_config_key: String,
    pub clone_args: Vec<String>,
}

pub fn clone_with_credential(
    remote_url: &str,
    git_clone_token: &str,
    destination: Option<&Path>,
) -> anyhow::Result<()> {
    let home = home_dir()?;
    let plan = git_clone_plan(remote_url, git_clone_token, destination, &home)?;
    let Some(parent) = plan.credential_store_path.parent() else {
        bail!("Scope Git credential store path has no parent directory");
    };
    fs::create_dir_all(parent).context("create Scope Git credential directory")?;
    restrict_directory_to_owner(parent);

    run_git(&[
        "config",
        "--global",
        "--replace-all",
        &plan.helper_config_key,
        &plan.helper_config_value,
    ])?;
    run_git(&[
        "config",
        "--global",
        "--replace-all",
        &plan.use_http_path_config_key,
        "true",
    ])?;
    approve_git_credential(&plan.credential_fields)?;
    let clone_args = plan
        .clone_args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    run_git(&clone_args)?;
    Ok(())
}

pub fn git_clone_plan(
    remote_url: &str,
    git_clone_token: &str,
    destination: Option<&Path>,
    home: &Path,
) -> anyhow::Result<GitClonePlan> {
    let credential_remote_url = remote_url_with_username(remote_url, GIT_CREDENTIAL_USERNAME)?;
    let url = Url::parse(&credential_remote_url).context("parse Scope Git remote URL")?;
    let credential_store_path = credential_store_path(home);
    let helper_config_value = format!(
        "store --file {}",
        quote_git_helper_arg(&git_path_for_config(&credential_store_path))
    );
    let mut clone_args = vec![
        "clone".to_string(),
        "-c".to_string(),
        GIT_PROACTIVE_AUTH_CONFIG.to_string(),
        credential_remote_url.clone(),
    ];
    if let Some(destination) = destination {
        clone_args.push(destination.to_string_lossy().to_string());
    }

    Ok(GitClonePlan {
        credential_remote_url,
        credential_fields: credential_fields(&url, GIT_CREDENTIAL_USERNAME, git_clone_token),
        credential_store_path,
        helper_config_key: credential_helper_config_key(&url),
        helper_config_value,
        use_http_path_config_key: credential_use_http_path_config_key(&url),
        clone_args,
    })
}

fn remote_url_with_username(remote_url: &str, username: &str) -> anyhow::Result<String> {
    let mut url = Url::parse(remote_url).context("parse Scope Git remote URL")?;
    url.set_username(username)
        .map_err(|_| anyhow::anyhow!("Scope Git remote URL cannot contain a username"))?;
    url.set_password(None)
        .map_err(|_| anyhow::anyhow!("Scope Git remote URL cannot contain a password"))?;
    Ok(url.to_string())
}

fn credential_fields(url: &Url, username: &str, password: &str) -> Vec<String> {
    vec![
        format!("protocol={}", url.scheme()),
        format!("host={}", url_host(url)),
        format!("path={}", credential_path(url)),
        format!("username={username}"),
        format!("password={password}"),
        String::new(),
    ]
}

fn credential_path(url: &Url) -> String {
    url.path().trim_start_matches('/').to_string()
}

fn credential_helper_config_key(url: &Url) -> String {
    format!("credential.{}://{}.helper", url.scheme(), url_host(url))
}

fn credential_use_http_path_config_key(url: &Url) -> String {
    format!(
        "credential.{}://{}.useHttpPath",
        url.scheme(),
        url_host(url)
    )
}

fn url_host(url: &Url) -> String {
    match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
        None => url.host_str().unwrap_or_default().to_string(),
    }
}

fn credential_store_path(home: &Path) -> PathBuf {
    home.join(".config").join("scope").join("git-credentials")
}

fn git_path_for_config(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn quote_git_helper_arg(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn home_dir() -> anyhow::Result<PathBuf> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .context("find home directory for Scope Git credential store")
}

fn approve_git_credential(fields: &[String]) -> anyhow::Result<()> {
    let mut child = Command::new("git")
        .args(["credential", "approve"])
        .stdin(Stdio::piped())
        .spawn()
        .context("start git credential approve")?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .context("open git credential approve stdin")?;
        for field in fields {
            stdin
                .write_all(field.as_bytes())
                .context("write Git credential field")?;
            stdin
                .write_all(b"\n")
                .context("write Git credential field")?;
        }
    }
    let status = child.wait().context("wait for git credential approve")?;
    if !status.success() {
        bail!("git credential approve failed");
    }
    Ok(())
}

fn run_git(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("git")
        .args(args)
        .status()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !status.success() {
        bail!("git {} failed", args.join(" "));
    }
    Ok(())
}

#[cfg(unix)]
fn restrict_directory_to_owner(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o700);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(not(unix))]
fn restrict_directory_to_owner(_path: &Path) {}
