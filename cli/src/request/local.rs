use crate::{
    api::{
        RepoSummaryResponse, RepositoryActor, RequestSummaryResponse,
        download_request_branch_bundle, get_repo,
    },
    git_repo::{
        GitRepo, branch_config_value, changed_paths_since_scope_base_at_commit, current_branch,
        fetch_scope_remote_with_bearer, push_head_to_ref_with_bearer, run_git_in_repo,
        set_branch_config_value,
    },
    push::DEFAULT_SCOPE_BRANCH,
    request::remote::{
        REQUEST_REMOTE_KEY, RequestRemoteTarget, load_request_remote, request_remote_name,
    },
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_core::domain::requests::RequestBaseAudience;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const REQUEST_ID_KEY: &str = "scopeRequestId";
const REQUEST_REF_KEY: &str = "scopeRequestRef";
const REQUEST_OWNER_KEY: &str = "scopeRequestOwner";
const REQUEST_REPO_KEY: &str = "scopeRequestRepo";
const REQUEST_BASE_AUDIENCE_KEY: &str = "scopeRequestBaseAudience";

pub(super) struct RequestContext {
    pub(super) target: RequestRemoteTarget,
    pub(super) repo: RepoSummaryResponse,
}

pub(super) fn load_context(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<&str>,
) -> anyhow::Result<RequestContext> {
    let remote = request_remote_name(git_repo, api_url, remote)?;
    let target = load_request_remote(git_repo, api_url, &remote)?;
    let repo = get_repo(client, api_url, session_token, &target.owner, &target.repo)?;
    Ok(RequestContext { target, repo })
}

pub(super) fn load_context_and_request_id(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    remote: Option<String>,
    request_id: Option<String>,
) -> anyhow::Result<(RequestContext, String)> {
    let context = load_context(git_repo, client, api_url, session_token, remote.as_deref())?;
    crate::request::render::print_repo_access(&context.repo);
    let request_id = current_or_explicit_request_id(git_repo, request_id)?;
    Ok((context, request_id))
}

pub(super) fn fetch_main_projection(
    git_repo: &GitRepo,
    context: &RequestContext,
    base_audience: RequestBaseAudience,
    session_token: &str,
) -> anyhow::Result<()> {
    let fetch_url = match base_audience {
        RequestBaseAudience::Public => &context.target.public_url,
        RequestBaseAudience::Private => &context.target.permissioned_url,
    };
    fetch_scope_remote_with_bearer(
        git_repo,
        fetch_url,
        &context.target.remote,
        DEFAULT_SCOPE_BRANCH,
        session_token,
    )
}

pub(super) fn normalized_submit_stake(
    repo: &RepoSummaryResponse,
    stake_credits: Option<u32>,
) -> anyhow::Result<Option<u32>> {
    if !repo.request_permissions.can_submit_request {
        bail!(
            "this user cannot submit requests to {}/{}",
            repo.owner_handle,
            repo.name
        );
    }
    match repo.access.actor {
        RepositoryActor::Public => match stake_credits {
            Some(stake) if stake > 0 => Ok(Some(stake)),
            _ => bail!("public request submission requires --stake-credits greater than 0"),
        },
        RepositoryActor::Member | RepositoryActor::Owner => {
            if stake_credits.unwrap_or(0) != 0 {
                bail!(
                    "owner/member request submission does not use credit stake; omit --stake-credits"
                );
            }
            Ok(None)
        }
    }
}

pub(super) fn push_request_head(
    target: &RequestRemoteTarget,
    session_token: &str,
    request_head_oid: &str,
    request_id: &str,
    request_ref: &str,
) -> anyhow::Result<()> {
    push_head_to_ref_with_bearer(
        &target.permissioned_url,
        request_head_oid,
        request_ref,
        session_token,
    )
    .with_context(|| format!("push request branch for {request_id}"))
}

pub(super) fn current_or_explicit_request_id(
    git_repo: &GitRepo,
    request_id: Option<String>,
) -> anyhow::Result<String> {
    maybe_current_or_explicit_request_id(git_repo, request_id)?
        .context("no request id supplied and current branch is not attached to a Scope request")
}

pub(super) fn maybe_current_or_explicit_request_id(
    git_repo: &GitRepo,
    request_id: Option<String>,
) -> anyhow::Result<Option<String>> {
    if let Some(request_id) = normalized_optional_arg(request_id) {
        return Ok(Some(request_id));
    }
    let branch = current_branch(git_repo)?;
    branch_config_value(git_repo, &branch, REQUEST_ID_KEY)
}

pub(super) fn maybe_request_branch_base_audience(
    git_repo: &GitRepo,
) -> anyhow::Result<Option<RequestBaseAudience>> {
    let branch = current_branch(git_repo)?;
    branch_config_value(git_repo, &branch, REQUEST_BASE_AUDIENCE_KEY)?
        .map(|value| parse_base_audience_config(&value))
        .transpose()
}

pub(super) fn request_branch_base_audience(
    git_repo: &GitRepo,
) -> anyhow::Result<RequestBaseAudience> {
    maybe_request_branch_base_audience(git_repo)?.context(
        "current branch is missing Scope request base audience; run scope request start first",
    )
}

pub(super) fn ensure_request_branch_context(
    git_repo: &GitRepo,
    command_name: &str,
) -> anyhow::Result<()> {
    let branch = current_branch(git_repo)?;
    if branch_config_value(git_repo, &branch, REQUEST_REMOTE_KEY)?.is_some()
        || branch_config_value(git_repo, &branch, REQUEST_ID_KEY)?.is_some()
    {
        return Ok(());
    }
    bail!("{command_name} requires a Scope request branch; run scope request start first")
}

pub(super) fn store_branch_context(
    git_repo: &GitRepo,
    branch: &str,
    context: &RequestContext,
) -> anyhow::Result<()> {
    set_branch_config_value(git_repo, branch, REQUEST_OWNER_KEY, &context.target.owner)?;
    set_branch_config_value(git_repo, branch, REQUEST_REPO_KEY, &context.target.repo)?;
    set_branch_config_value(git_repo, branch, REQUEST_REMOTE_KEY, &context.target.remote)?;
    Ok(())
}

pub(super) fn store_request_metadata(
    git_repo: &GitRepo,
    branch: &str,
    context: &RequestContext,
    request: &RequestSummaryResponse,
) -> anyhow::Result<()> {
    store_branch_context(git_repo, branch, context)?;
    store_request_metadata_fields(
        git_repo,
        branch,
        &request.id,
        &request.request_ref,
        request.base_audience,
    )
}

pub(super) fn store_request_metadata_fields(
    git_repo: &GitRepo,
    branch: &str,
    request_id: &str,
    request_ref: &str,
    base_audience: RequestBaseAudience,
) -> anyhow::Result<()> {
    set_branch_config_value(git_repo, branch, REQUEST_ID_KEY, request_id)?;
    set_branch_config_value(git_repo, branch, REQUEST_REF_KEY, request_ref)?;
    set_branch_config_value(
        git_repo,
        branch,
        REQUEST_BASE_AUDIENCE_KEY,
        base_audience_config_value(base_audience),
    )?;
    Ok(())
}

pub(super) fn track_request_branch_ref(
    git_repo: &GitRepo,
    branch: &str,
    target: &RequestRemoteTarget,
    request_id: &str,
    request_head_oid: &str,
) -> anyhow::Result<()> {
    let remote_ref = request_remote_ref(&target.remote, request_id);
    run_git_in_repo(git_repo, &["update-ref", &remote_ref, request_head_oid])?;
    set_branch_config_value(git_repo, branch, "remote", &target.remote)?;
    set_branch_config_value(
        git_repo,
        branch,
        "merge",
        &format!("refs/heads/scope/requests/{request_id}"),
    )
}

pub(super) fn print_change_summary(
    git_repo: &GitRepo,
    target: &RequestRemoteTarget,
    request_head_oid: &str,
) -> anyhow::Result<()> {
    let remote_main = remote_main_ref(&target.remote);
    let changes =
        changed_paths_since_scope_base_at_commit(git_repo, Some(&remote_main), request_head_oid)?;
    if changes.is_empty() {
        println!("Committed diff: no file changes from {remote_main}");
        return Ok(());
    }

    println!(
        "Committed diff: {} changed file(s) from {remote_main}",
        changes.len()
    );
    for change in changes.iter().take(12) {
        println!("  {} {}", change.status, change.path);
    }
    if changes.len() > 12 {
        println!("  ... {} more", changes.len() - 12);
    }
    Ok(())
}

pub(super) fn fetch_request_branch_bundle(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    context: &RequestContext,
    request: &RequestSummaryResponse,
) -> anyhow::Result<()> {
    let bytes = download_request_branch_bundle(
        client,
        api_url,
        session_token,
        &context.target.owner,
        &context.target.repo,
        &request.id,
    )?;
    let bundle = TemporaryFile::new(request_bundle_path(git_repo, &request.id)?);
    if let Some(parent) = bundle.path().parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.to_string_lossy()))?;
    }
    fs::write(bundle.path(), bytes)
        .with_context(|| format!("write {}", bundle.path().to_string_lossy()))?;
    let remote_ref = request_remote_ref(&context.target.remote, &request.id);
    run_git_in_repo(
        git_repo,
        &[
            "fetch",
            bundle.path().to_string_lossy().as_ref(),
            &format!("+{}:{remote_ref}", request.request_ref),
        ],
    )
}

struct TemporaryFile {
    path: PathBuf,
}

impl TemporaryFile {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(super) fn projection_label_for_repo(repo: &RepoSummaryResponse) -> &'static str {
    match repo.access.actor {
        RepositoryActor::Owner | RepositoryActor::Member => "private main",
        RepositoryActor::Public => "public main",
    }
}

pub(super) fn remote_main_ref(remote: &str) -> String {
    format!("refs/remotes/{remote}/{DEFAULT_SCOPE_BRANCH}")
}

pub(super) fn request_remote_ref(remote: &str, request_id: &str) -> String {
    format!("refs/remotes/{remote}/scope/requests/{request_id}")
}

pub(super) fn default_request_branch_name() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("scope/request/{now}")
}

pub(super) fn default_join_branch_name(request_id: &str) -> String {
    format!("scope/request/{request_id}")
}

pub(super) fn base_audience_for_repo(repo: &RepoSummaryResponse) -> RequestBaseAudience {
    match repo.access.actor {
        RepositoryActor::Owner | RepositoryActor::Member => RequestBaseAudience::Private,
        RepositoryActor::Public => RequestBaseAudience::Public,
    }
}

fn base_audience_config_value(audience: RequestBaseAudience) -> &'static str {
    match audience {
        RequestBaseAudience::Public => "public",
        RequestBaseAudience::Private => "private",
    }
}

pub(super) fn parse_base_audience_config(value: &str) -> anyhow::Result<RequestBaseAudience> {
    match value {
        "public" => Ok(RequestBaseAudience::Public),
        "private" => Ok(RequestBaseAudience::Private),
        _ => bail!("invalid Scope request base audience '{value}'"),
    }
}

fn request_bundle_path(git_repo: &GitRepo, request_id: &str) -> anyhow::Result<PathBuf> {
    if request_id.chars().any(char::is_control) {
        bail!("request id cannot contain control characters");
    }
    Ok(git_repo
        .root
        .join(".scope")
        .join("tmp")
        .join(format!("{request_id}.bundle.tmp")))
}

fn normalized_optional_arg(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
