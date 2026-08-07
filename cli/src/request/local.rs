use crate::{
    api::{RepoSummaryResponse, RequestSummaryResponse, get_repo, list_requests},
    git_repo::{
        GitRepo, branch_config_value, current_branch, fetch_scope_remote_with_bearer,
        push_head_to_ref_with_bearer, run_git_in_repo, scope_remote_head_oid,
        set_branch_config_value,
    },
    push::DEFAULT_SCOPE_BRANCH,
    request::remote::{
        REQUEST_REMOTE_KEY, RequestRemoteTarget, load_request_remote, request_remote_name,
    },
};
use anyhow::{Context, bail};
use reqwest::blocking::Client;
use scope_api_contract::RequestAudience;

const REQUEST_ID_KEY: &str = "scopeRequestId";
const REQUEST_OWNER_KEY: &str = "scopeRequestOwner";
const REQUEST_REPO_KEY: &str = "scopeRequestRepo";
const REQUEST_AUDIENCE_KEY: &str = "scopeRequestAudience";

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
    let request_id = request_id_for_context(
        git_repo,
        client,
        api_url,
        session_token,
        &context,
        request_id,
    )?;
    Ok((context, request_id))
}

pub(super) fn refresh_main_projection(
    git_repo: &GitRepo,
    target: &RequestRemoteTarget,
    audience: RequestAudience,
    session_token: &str,
) -> anyhow::Result<String> {
    let fetch_url = match audience {
        RequestAudience::Public => &target.public_url,
        RequestAudience::Private => &target.permissioned_url,
    };
    fetch_scope_remote_with_bearer(
        git_repo,
        fetch_url,
        &target.remote,
        DEFAULT_SCOPE_BRANCH,
        session_token,
    )?;
    scope_remote_head_oid(git_repo, &target.remote, DEFAULT_SCOPE_BRANCH)?
        .context("Scope main projection did not produce a local remote ref")
}

pub(super) fn push_request_head(
    target: &RequestRemoteTarget,
    session_token: &str,
    request_head_oid: &str,
    request_id: &str,
    request_name: &str,
) -> anyhow::Result<()> {
    let request_ref = format!("refs/heads/{request_name}");
    push_head_to_ref_with_bearer(
        &target.permissioned_url,
        request_head_oid,
        &request_ref,
        session_token,
    )
    .with_context(|| format!("push request branch for {request_id}"))
}

pub(super) fn request_id_for_context(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    context: &RequestContext,
    request_id: Option<String>,
) -> anyhow::Result<String> {
    maybe_request_id_for_context(
        git_repo,
        client,
        api_url,
        session_token,
        context,
        request_id,
    )?
    .context("current branch is not a visible Scope request; switch to origin/<request-name> or pass a request id")
}

pub(super) fn maybe_request_id_for_context(
    git_repo: &GitRepo,
    client: &Client,
    api_url: &str,
    session_token: &str,
    context: &RequestContext,
    request_id: Option<String>,
) -> anyhow::Result<Option<String>> {
    let explicit = normalized_optional_arg(request_id);
    if let Some(request_id) = explicit
        .as_deref()
        .filter(|value| value.starts_with("req_"))
    {
        return Ok(Some(request_id.to_string()));
    }
    let branch = current_branch(git_repo)?;
    if explicit.is_none()
        && let Some(request_id) = branch_config_value(git_repo, &branch, REQUEST_ID_KEY)?
    {
        validate_stored_request_target(git_repo, &branch, context)?;
        return Ok(Some(request_id));
    }
    let request_name = match explicit {
        Some(request_name) => request_name,
        None => {
            let tracking_remote = branch_config_value(git_repo, &branch, "remote")?;
            let merge_ref = branch_config_value(git_repo, &branch, "merge")?;
            inferred_request_name(
                branch,
                &context.target.remote,
                tracking_remote.as_deref(),
                merge_ref.as_deref(),
            )
        }
    };
    let mut cursor = None;
    loop {
        let page = list_requests(
            client,
            api_url,
            session_token,
            &context.target.owner,
            &context.target.repo,
            cursor.as_deref(),
        )?;
        if let Some(request) = page
            .requests
            .into_iter()
            .find(|request| request.name == request_name)
        {
            return Ok(Some(request.id));
        }
        let Some(next_cursor) = page.next_cursor else {
            return Ok(None);
        };
        cursor = Some(next_cursor);
    }
}

fn inferred_request_name(
    branch: String,
    selected_remote: &str,
    tracking_remote: Option<&str>,
    merge_ref: Option<&str>,
) -> String {
    if tracking_remote == Some(selected_remote)
        && let Some(request_name) =
            merge_ref.and_then(|request_ref| request_ref.strip_prefix("refs/heads/"))
    {
        return request_name.to_string();
    }
    branch
}

fn validate_stored_request_target(
    git_repo: &GitRepo,
    branch: &str,
    context: &RequestContext,
) -> anyhow::Result<()> {
    let stored_owner = branch_config_value(git_repo, branch, REQUEST_OWNER_KEY)?;
    let stored_repo = branch_config_value(git_repo, branch, REQUEST_REPO_KEY)?;
    match (stored_owner.as_deref(), stored_repo.as_deref()) {
        (Some(owner), Some(repo))
            if owner == context.target.owner && repo == context.target.repo =>
        {
            Ok(())
        }
        (Some(owner), Some(repo)) => bail!(
            "current branch belongs to Scope repository {owner}/{repo}, but remote {} targets {}/{}; pass the correct --remote",
            context.target.remote,
            context.target.owner,
            context.target.repo
        ),
        _ => bail!(
            "current branch request metadata is incomplete; pass --request <name-or-id> explicitly"
        ),
    }
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
    store_request_metadata_fields(git_repo, branch, &request.id, request.audience)
}

pub(super) fn store_request_metadata_fields(
    git_repo: &GitRepo,
    branch: &str,
    request_id: &str,
    audience: RequestAudience,
) -> anyhow::Result<()> {
    set_branch_config_value(git_repo, branch, REQUEST_ID_KEY, request_id)?;
    set_branch_config_value(
        git_repo,
        branch,
        REQUEST_AUDIENCE_KEY,
        audience_config_value(audience),
    )?;
    Ok(())
}

pub(super) fn track_request_branch_ref(
    git_repo: &GitRepo,
    branch: &str,
    target: &RequestRemoteTarget,
    request_name: &str,
    request_head_oid: &str,
) -> anyhow::Result<()> {
    let remote_ref = request_remote_ref(&target.remote, request_name);
    run_git_in_repo(git_repo, &["update-ref", &remote_ref, request_head_oid])?;
    set_branch_config_value(git_repo, branch, "remote", &target.remote)?;
    set_branch_config_value(
        git_repo,
        branch,
        "merge",
        &format!("refs/heads/{request_name}"),
    )
}

pub(super) fn projection_label_for_audience(audience: RequestAudience) -> &'static str {
    match audience {
        RequestAudience::Public => "public main",
        RequestAudience::Private => "private main",
    }
}

pub(super) fn remote_main_ref(remote: &str) -> String {
    format!("refs/remotes/{remote}/{DEFAULT_SCOPE_BRANCH}")
}

pub(super) fn request_remote_ref(remote: &str, request_name: &str) -> String {
    format!("refs/remotes/{remote}/{request_name}")
}

fn audience_config_value(audience: RequestAudience) -> &'static str {
    match audience {
        RequestAudience::Public => "public",
        RequestAudience::Private => "private",
    }
}

fn normalized_optional_arg(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
#[cfg(test)]
mod tests {
    use super::{inferred_request_name, refresh_main_projection};
    use crate::{
        git_repo::GitRepo,
        git_transport::{GitAccess, ScopeRemote},
        test_support::TestDir,
    };
    use scope_api_contract::RequestAudience;
    use std::fs;

    #[test]
    fn merge_ref_only_names_a_request_for_the_selected_scope_remote() {
        assert_eq!(
            inferred_request_name(
                "scope-fix".to_string(),
                "scope",
                Some("scope"),
                Some("refs/heads/request-fix"),
            ),
            "request-fix"
        );
        assert_eq!(
            inferred_request_name(
                "scope-fix".to_string(),
                "scope",
                Some("origin"),
                Some("refs/heads/other-fix"),
            ),
            "scope-fix"
        );
    }

    #[test]
    fn main_projection_refresh_follows_alternating_request_audiences() {
        let (public, public_oid) = repository_with_commit("public-main", "public.txt");
        let (private, private_oid) = repository_with_commit("private-main", "private.txt");
        let checkout = TestDir::git_repo("alternating-main", "main");
        let repo = GitRepo {
            root: checkout.path().to_path_buf(),
        };
        let target = ScopeRemote {
            remote: "scope".to_string(),
            access: GitAccess::Permissioned,
            public_url: file_url(public.path()),
            permissioned_url: file_url(private.path()),
            owner: "owner".to_string(),
            repo: "repo".to_string(),
        };

        assert_eq!(
            refresh_main_projection(&repo, &target, RequestAudience::Public, "unused").unwrap(),
            public_oid
        );
        assert_eq!(
            refresh_main_projection(&repo, &target, RequestAudience::Private, "unused").unwrap(),
            private_oid
        );
        assert_eq!(
            refresh_main_projection(&repo, &target, RequestAudience::Public, "unused").unwrap(),
            public_oid
        );
    }

    fn repository_with_commit(label: &str, file: &str) -> (TestDir, String) {
        let dir = TestDir::git_repo(label, "main");
        dir.run_git(["config", "user.email", "scope@example.test"]);
        dir.run_git(["config", "user.name", "Scope Test"]);
        fs::write(dir.path().join(file), format!("{label}\n")).unwrap();
        dir.run_git(["add", file]);
        dir.run_git(["commit", "-m", label]);
        let oid = String::from_utf8(dir.run_git(["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();
        (dir, oid)
    }

    fn file_url(path: &std::path::Path) -> String {
        format!("file://{}", path.display())
    }
}
