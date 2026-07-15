use crate::{
    api::{
        CreatePushIntentParams, RepoPublicationState, RepositoryAccessResponse, RepositoryActor,
        api_url, create_push_intent, get_repo_config, http_client,
    },
    git_repo::{
        GitRepo, changed_paths_since_scope_base_at_commit, ensure_git_repo_ready,
        fetch_scope_remote_with_bearer, git_remote_push_url, head_oid, mark_scope_remote_pushed,
        push_head_with_bearer, scope_remote_head_oid, warn_if_dirty_working_tree,
    },
    git_transport::{GitAccess, ScopeRemote, select_scope_push_remote},
    login::session_from_cache_or_browser,
    repo_config::{
        ensure_scope_repo_config_exists, load_worktree_scope_repo_config,
        load_worktree_scope_repo_config_base_hash, mark_worktree_scope_repo_config_synced,
        repo_config_path, write_worktree_scope_repo_config_with_base,
    },
    review::{ensure_review_terminal_available, run_push_review},
};
use anyhow::bail;
use scope_core::domain::repo_config::repo_config_fingerprint;
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_SCOPE_BRANCH: &str = "main";

#[derive(Debug, Eq, PartialEq)]
pub struct ScopePushOutcome {
    pub owner: String,
    pub repo: String,
}

pub fn run(explicit_remote: Option<&str>, no_review: bool) -> anyhow::Result<()> {
    let git_repo = ensure_git_repo_ready("scope push")?;
    let reviewed_head_oid = head_oid(&git_repo)?;
    let config_created = ensure_scope_repo_config_exists(&git_repo.root)?;
    let mut config = load_worktree_scope_repo_config(&git_repo.root)?;
    warn_if_dirty_working_tree(&git_repo)?;
    if !no_review {
        ensure_review_terminal_available("scope push review")?;
    }

    let api_url = api_url();
    let remote = select_scope_push_remote(&git_repo, &api_url, explicit_remote)?;
    let target = load_scope_remote(&git_repo, &api_url, &remote)?;
    let client = http_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    let push_context = get_repo_config(
        &client,
        &api_url,
        &session.token,
        &target.owner,
        &target.repo,
    )?;
    ensure_scope_remote_can_receive_push(
        &target,
        push_context.lifecycle_state,
        &push_context.access,
    )?;
    if config_created {
        write_worktree_scope_repo_config_with_base(&git_repo.root, &push_context.config)?;
        config = push_context.config.clone();
        eprintln!("Created {}", repo_config_path());
    } else {
        let local_config_hash = repo_config_fingerprint(&config)?;
        match load_worktree_scope_repo_config_base_hash(&git_repo.root) {
            Ok(hash) if hash == push_context.config_hash => {}
            Ok(_) if local_config_hash == push_context.config_hash => {
                mark_worktree_scope_repo_config_synced(&git_repo.root, &config)?;
            }
            Ok(hash) if hash == local_config_hash => {
                write_worktree_scope_repo_config_with_base(&git_repo.root, &push_context.config)?;
                config = push_context.config.clone();
                eprintln!(
                    "Scope repo config changed; refreshed {}",
                    repo_config_path()
                );
            }
            Ok(_) => bail!(
                "Scope repo config changed, and local {} has unsynced edits. Run scope review, resolve the config, then retry scope push.",
                repo_config_path()
            ),
            Err(_) if local_config_hash == push_context.config_hash => {
                mark_worktree_scope_repo_config_synced(&git_repo.root, &config)?;
            }
            Err(error) => bail!(
                "{error}. Local {} has unsynced edits, so Scope will not overwrite it.",
                repo_config_path()
            ),
        }
    }
    let local_remote_head = scope_remote_head_oid(&git_repo, &remote, DEFAULT_SCOPE_BRANCH)?;
    if push_context.lifecycle_state == RepoPublicationState::Published
        && local_remote_head.as_deref() != push_context.head_oid.as_deref()
    {
        fetch_scope_remote_with_bearer(
            &git_repo,
            &target.permissioned_url,
            &remote,
            DEFAULT_SCOPE_BRANCH,
            &session.token,
        )?;
    }
    let reviewed_base_oid = if no_review {
        None
    } else if push_context.lifecycle_state == RepoPublicationState::Published {
        Some(scope_remote_head_oid(
            &git_repo,
            &remote,
            DEFAULT_SCOPE_BRANCH,
        )?)
    } else {
        Some(None)
    };
    if let Some(review_base_oid) = &reviewed_base_oid {
        let changed_paths = changed_paths_since_scope_base_at_commit(
            &git_repo,
            review_base_oid.as_deref(),
            &reviewed_head_oid,
        )?;
        config = run_push_review(&git_repo, &reviewed_head_oid, &changed_paths)?;
    }
    let base_config_hash = load_worktree_scope_repo_config_base_hash(&git_repo.root)?;
    let intent = create_push_intent(
        &client,
        &api_url,
        &session.token,
        CreatePushIntentParams {
            owner: &target.owner,
            repo: &target.repo,
            head_oid: &reviewed_head_oid,
            base_config_hash: &base_config_hash,
            config: &config,
        },
    )?;
    if let Some(review_base_oid) = &reviewed_base_oid {
        ensure_reviewed_base_matches_intent(
            review_base_oid.as_deref(),
            intent.base_head_oid.as_deref(),
        )?;
    }
    ensure_review_base_matches_intent(
        &git_repo,
        &target.permissioned_url,
        &remote,
        &session.token,
        intent.base_head_oid.as_deref(),
    )?;
    ensure_push_intent_not_expired(intent.expires_at_unix)?;

    let outcome = match push_reviewed_head_with_intent(
        &session.token,
        &target,
        &reviewed_head_oid,
        &intent.token,
    ) {
        Ok(outcome) => outcome,
        Err(_) if push_intent_expired(intent.expires_at_unix) => {
            bail!("Scope push review expired; rerun scope push")
        }
        Err(error) => return Err(error),
    };
    mark_scope_remote_pushed(&git_repo, &remote, DEFAULT_SCOPE_BRANCH, &reviewed_head_oid)?;
    mark_worktree_scope_repo_config_synced(&git_repo.root, &config)?;
    println!(
        "Pushed to Scope: {}/{}\nPush applied by Scope.",
        outcome.owner, outcome.repo
    );
    Ok(())
}

fn ensure_push_intent_not_expired(expires_at_unix: u64) -> anyhow::Result<()> {
    if push_intent_expired(expires_at_unix) {
        bail!("Scope push review expired; rerun scope push");
    }
    Ok(())
}

fn push_intent_expired(expires_at_unix: u64) -> bool {
    unix_now() >= expires_at_unix
}

fn ensure_review_base_matches_intent(
    git_repo: &GitRepo,
    push_url: &str,
    remote: &str,
    session_token: &str,
    intent_base_head_oid: Option<&str>,
) -> anyhow::Result<()> {
    let Some(intent_base_head_oid) = intent_base_head_oid else {
        return Ok(());
    };
    if scope_remote_head_oid(git_repo, remote, DEFAULT_SCOPE_BRANCH)?.as_deref()
        == Some(intent_base_head_oid)
    {
        return Ok(());
    }

    fetch_scope_remote_with_bearer(
        git_repo,
        push_url,
        remote,
        DEFAULT_SCOPE_BRANCH,
        session_token,
    )?;
    if scope_remote_head_oid(git_repo, remote, DEFAULT_SCOPE_BRANCH)?.as_deref()
        == Some(intent_base_head_oid)
    {
        return Ok(());
    }
    bail!("Scope changed while preparing push review; rerun scope push")
}

fn ensure_reviewed_base_matches_intent(
    reviewed_base_oid: Option<&str>,
    intent_base_head_oid: Option<&str>,
) -> anyhow::Result<()> {
    if reviewed_base_oid != intent_base_head_oid {
        bail!("Scope changed while preparing push review; rerun scope push");
    }
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn load_scope_remote(
    git_repo: &GitRepo,
    api_url: &str,
    remote: &str,
) -> anyhow::Result<ScopeRemote> {
    let push_url = git_remote_push_url(git_repo, remote)?;
    let target = ScopeRemote::parse(api_url, remote, &push_url)?;
    if target.access != GitAccess::Permissioned {
        bail!("Scope remote must have path /git/permissioned/owner/repo");
    }
    Ok(target)
}

pub fn ensure_scope_remote_can_receive_push(
    target: &ScopeRemote,
    lifecycle_state: RepoPublicationState,
    access: &RepositoryAccessResponse,
) -> anyhow::Result<()> {
    if lifecycle_state == RepoPublicationState::Unpublished {
        ensure_unpublished_repo_can_receive_first_push(&target.owner, &target.repo, access.actor)
    } else {
        ensure_published_repo_can_receive_push(
            &target.owner,
            &target.repo,
            lifecycle_state,
            access.can_push,
        )
    }
}

pub fn push_reviewed_head_with_intent(
    session_token: &str,
    target: &ScopeRemote,
    reviewed_head_oid: &str,
    push_intent_token: &str,
) -> anyhow::Result<ScopePushOutcome> {
    push_head_with_bearer(
        &target.permissioned_url,
        reviewed_head_oid,
        DEFAULT_SCOPE_BRANCH,
        session_token,
        push_intent_token,
    )?;

    Ok(ScopePushOutcome {
        owner: target.owner.clone(),
        repo: target.repo.clone(),
    })
}

fn ensure_unpublished_repo_can_receive_first_push(
    owner: &str,
    repo: &str,
    actor: RepositoryActor,
) -> anyhow::Result<()> {
    if actor != RepositoryActor::Owner {
        bail!("you do not have owner access to first-push {owner}/{repo}");
    }
    Ok(())
}

fn ensure_published_repo_can_receive_push(
    owner: &str,
    repo: &str,
    lifecycle_state: RepoPublicationState,
    can_push: bool,
) -> anyhow::Result<()> {
    match lifecycle_state {
        RepoPublicationState::Unpublished => {
            bail!("repo {owner}/{repo} is waiting for its first push. Run: scope init");
        }
        RepoPublicationState::Published => {}
    }

    if !can_push {
        bail!("you do not have write access to {owner}/{repo}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_push_intent_reports_rerun_message() {
        let error = ensure_push_intent_not_expired(0).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Scope push review expired; rerun scope push")
        );
        ensure_push_intent_not_expired(unix_now().saturating_add(60)).unwrap();
    }

    #[test]
    fn reviewed_base_must_match_push_intent_base() {
        ensure_reviewed_base_matches_intent(None, None).unwrap();
        ensure_reviewed_base_matches_intent(Some("abc"), Some("abc")).unwrap();
        let error = ensure_reviewed_base_matches_intent(Some("abc"), Some("def")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Scope changed while preparing push review; rerun scope push")
        );
        assert!(ensure_reviewed_base_matches_intent(None, Some("def")).is_err());
        assert!(ensure_reviewed_base_matches_intent(Some("abc"), None).is_err());
    }

    #[test]
    fn first_push_requires_owner_access() {
        ensure_unpublished_repo_can_receive_first_push("owner", "repo", RepositoryActor::Owner)
            .unwrap();
        for actor in [RepositoryActor::Member, RepositoryActor::Public] {
            assert!(
                ensure_unpublished_repo_can_receive_first_push("owner", "repo", actor).is_err()
            );
        }
    }

    #[test]
    fn published_push_requires_write_access() {
        for (state, can_push, allowed) in [
            (RepoPublicationState::Published, true, true),
            (RepoPublicationState::Published, false, false),
            (RepoPublicationState::Unpublished, true, false),
        ] {
            assert_eq!(
                ensure_published_repo_can_receive_push("owner", "repo", state, can_push).is_ok(),
                allowed,
            );
        }
    }
}
