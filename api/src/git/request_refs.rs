use crate::{
    config::{DEFAULT_GIT_BRANCH, EMPTY_GIT_OID},
    error::ApiError,
    git::{
        import::{git_snapshot_from_ref, run_git, run_git_output, validate_pushed_commit_range},
        request_ref_public_safety::ensure_public_request_ref_is_public_safe,
        storage::{
            receive_pack_staging_repo_path, remove_dir_if_exists, request_ref_store_repo_path,
            write_receive_pack_hook,
        },
    },
    state::AppState,
};
use scope_domain::{
    content::SourceBlob,
    repository::{Repository, RepositoryIncarnation},
    requests::{Request, RequestAudience, canonical_request_ref},
};
use scope_object_store::source_blob_bytes;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path as FsPath, PathBuf},
};

mod locks;
mod revision;
#[cfg(test)]
use crate::persistence::unix_now;
use locks::acquire_request_ref_store_lock;
pub(crate) use locks::acquire_request_ref_update_lock;
#[cfg(test)]
use locks::git_lock_is_stale;
pub(crate) use revision::with_request_revision_store_repo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestRefUpdate {
    pub(crate) request_ref: String,
    pub(crate) request_name: String,
    pub(crate) old_head_oid: Option<String>,
    pub(crate) new_head_oid: String,
}

pub(crate) fn is_request_ref(refname: &str) -> bool {
    request_name_from_ref(refname)
        .is_some_and(|name| scope_domain::requests::validate_request_name(name).is_ok())
}

fn request_name_from_ref(refname: &str) -> Option<&str> {
    let name = refname.strip_prefix("refs/heads/")?;
    (!name.is_empty() && name != DEFAULT_GIT_BRANCH && !name.contains('/')).then_some(name)
}

fn is_request_ref_candidate(refname: &str) -> bool {
    request_name_from_ref(refname).is_some()
}

pub(crate) fn receive_pack_refs(staging_repo: &FsPath) -> Result<Vec<(String, String)>, ApiError> {
    refs_for_prefixes(
        staging_repo,
        &["refs/heads", "refs/tags"],
        "reading receive-pack refs",
    )
}

pub(crate) fn request_ref_update_from_refs(
    refs_before: &[(String, String)],
    refs_after: &[(String, String)],
) -> Result<Option<RequestRefUpdate>, ApiError> {
    let before = refs_by_name(refs_before);
    let after = refs_by_name(refs_after);
    let mut changed = Vec::new();

    for refname in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
        if !is_request_ref_candidate(refname) {
            continue;
        }
        let old = before.get(refname);
        let new = after.get(refname);
        if old == new {
            continue;
        }
        let Some(new_head_oid) = new else {
            return Err(ApiError::bad_request(
                "Scope does not accept request branch deletes",
            ));
        };
        let request_name =
            request_name_from_ref(refname).expect("request ref was classified above");
        if !is_request_ref(refname) {
            scope_domain::requests::validate_request_name(request_name).map_err(|error| {
                ApiError::bad_request(format!(
                    "invalid request branch '{request_name}': {}",
                    error.message
                ))
            })?;
        }
        changed.push(RequestRefUpdate {
            request_ref: refname.clone(),
            request_name: request_name.to_string(),
            old_head_oid: old.cloned(),
            new_head_oid: new_head_oid.clone(),
        });
    }

    match changed.len() {
        0 => Ok(None),
        1 => Ok(changed.pop()),
        _ => Err(ApiError::bad_request(
            "Scope accepts exactly one request ref update",
        )),
    }
}

pub(crate) fn non_request_refs_changed(
    refs_before: &[(String, String)],
    refs_after: &[(String, String)],
) -> bool {
    let before = refs_by_name(refs_before);
    let after = refs_by_name(refs_after);
    before
        .keys()
        .chain(after.keys())
        .filter(|refname| !is_request_ref_candidate(refname))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .any(|refname| before.get(refname) != after.get(refname))
}

pub(crate) fn create_request_receive_pack_staging_repo(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    seed_repo: &FsPath,
) -> Result<PathBuf, ApiError> {
    let repo_root = receive_pack_staging_repo_path(state, incarnation)?;
    if let Some(parent) = repo_root.parent() {
        crate::persistence::ensure_private_dir(parent)?;
    }
    run_git(
        None,
        &[
            "clone",
            "--bare",
            "--no-hardlinks",
            seed_repo.to_string_lossy().as_ref(),
            repo_root.to_string_lossy().as_ref(),
        ],
        "cloning request receive-pack staging repo",
    )?;
    if let Err(error) = run_git(
        Some(&repo_root),
        &["config", "http.receivepack", "true"],
        "enabling request receive-pack",
    ) {
        let _ = fs::remove_dir_all(&repo_root);
        return Err(error);
    }
    Ok(repo_root)
}

pub(crate) fn install_request_receive_pack_hook(repo_root: &FsPath) -> Result<(), ApiError> {
    install_request_pre_receive_hook(repo_root)
}

/// Adds every already-authorized request snapshot to a disposable upload-pack repository.
/// The caller chooses the visible requests; this function never reaches into the private main
/// repository or advertises any other durable request-store refs.
pub(crate) fn attach_visible_request_refs(
    state: &AppState,
    requests: &[Request],
    target_repo: &FsPath,
    public_base_repo: Option<&FsPath>,
) -> Result<(), ApiError> {
    for request in requests {
        let request_ref = canonical_request_ref(&request.name);
        if let Some(snapshot) = request.git_snapshot.as_ref() {
            let bundle_path = target_repo.with_extension(format!(
                "read-view-{}.bundle.tmp",
                hex::encode(
                    &Sha256::digest(format!("{}:{}", request.name, snapshot.sha256).as_bytes())
                        [..8]
                )
            ));
            let bytes = source_blob_bytes(state.object_store.as_ref(), snapshot)?;
            fs::write(&bundle_path, bytes).map_err(ApiError::internal)?;
            let bundle = bundle_path.to_string_lossy().to_string();
            let refspec = format!("+{request_ref}:{request_ref}");
            let result = run_git(
                Some(target_repo),
                &["fetch", &bundle, &refspec],
                "attaching request ref to Git read view",
            );
            let _ = fs::remove_file(&bundle_path);
            result?;
        } else {
            // A newly started request initially points at its selected main base and therefore
            // needs no snapshot object transfer.
            if !request_ref_oid_is_commit(target_repo, &request.head_oid)?
                && let Some(public_base_repo) = public_base_repo
            {
                let temporary_ref = "refs/scope/internal/public-request-base";
                let refspec = format!("+refs/heads/{DEFAULT_GIT_BRANCH}:{temporary_ref}");
                run_git(
                    Some(target_repo),
                    &[
                        "fetch",
                        public_base_repo.to_string_lossy().as_ref(),
                        &refspec,
                    ],
                    "attaching public request base to Git read view",
                )?;
                run_git(
                    Some(target_repo),
                    &["update-ref", "-d", temporary_ref],
                    "removing temporary public request base ref",
                )?;
            }
            if !request_ref_oid_is_commit(target_repo, &request.head_oid)? {
                tracing::warn!(
                    request_id = request.id,
                    request_name = request.name,
                    head_oid = request.head_oid,
                    "omitting snapshotless request whose base commit is unavailable in Git read view"
                );
                continue;
            }
            run_git(
                Some(target_repo),
                &["update-ref", &request_ref, &request.head_oid],
                "attaching unmodified request ref to Git read view",
            )?;
        }
        let attached_head = request_ref_head(target_repo, &request_ref)?;
        if attached_head.as_deref() != Some(request.head_oid.as_str()) {
            return Err(ApiError::infrastructure_unavailable(
                "request snapshot does not match request metadata",
            ));
        }
    }
    Ok(())
}

pub(crate) fn delete_request_ref_from_store(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request_ref: &str,
) -> Result<(), ApiError> {
    let _update_lock = acquire_request_ref_update_lock(state, incarnation, request_ref)?;
    let store_repo = request_ref_store_repo_path(state, incarnation);
    if !store_repo.exists() {
        return Ok(());
    }
    let _store_lock = acquire_request_ref_store_lock(state, incarnation)?;
    if request_ref_exists(&store_repo, request_ref)? {
        run_git(
            Some(&store_repo),
            &["update-ref", "-d", request_ref],
            "deleting request ref",
        )?;
    }
    Ok(())
}

fn refs_for_prefixes(
    repo: &FsPath,
    prefixes: &[&str],
    action: &str,
) -> Result<Vec<(String, String)>, ApiError> {
    let mut args = vec!["for-each-ref", "--format=%(refname)%00%(objectname)"];
    args.extend(prefixes.iter().copied());
    let output = run_git_output(Some(repo), &args, action)?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8(output.stdout).map_err(ApiError::bad_request)?;
    text.lines()
        .map(|line| {
            let (refname, oid) = line
                .split_once('\0')
                .ok_or_else(|| ApiError::internal_message("invalid git ref listing"))?;
            Ok((refname.to_string(), oid.to_string()))
        })
        .collect()
}

fn refs_by_name(refs: &[(String, String)]) -> BTreeMap<String, String> {
    refs.iter()
        .map(|(refname, oid)| (refname.clone(), oid.clone()))
        .collect()
}

fn install_request_pre_receive_hook(repo_root: &FsPath) -> Result<(), ApiError> {
    let hook = repo_root.join("hooks").join("pre-receive");
    let script = format!(
        "#!/bin/sh\ncount=0\nwhile read old new ref; do\n  count=$((count + 1))\n  case \"$ref\" in\n    refs/heads/{DEFAULT_GIT_BRANCH})\n      echo \"Scope contributors cannot update main\" >&2\n      exit 1\n      ;;\n    refs/heads/*) ;;\n    *)\n      echo \"Scope request pushes only accept named request branches\" >&2\n      exit 1\n      ;;\n  esac\n  if [ \"$new\" = \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope does not accept request branch deletes\" >&2\n    exit 1\n  fi\n  if [ \"$(git cat-file -t \"$new\" 2>/dev/null)\" != \"commit\" ]; then\n    echo \"Scope request refs must point at commits\" >&2\n    exit 1\n  fi\n  if [ \"$old\" != \"{EMPTY_GIT_OID}\" ] && ! git merge-base --is-ancestor \"$old\" \"$new\"; then\n    echo \"Scope rejects non-fast-forward request pushes\" >&2\n    exit 1\n  fi\ndone\nif [ \"$count\" -ne 1 ]; then\n  echo \"Scope accepts exactly one request ref update\" >&2\n  exit 1\nfi\n"
    );
    write_receive_pack_hook(&hook, &script)
}

pub(crate) struct PersistedRequestRef {
    pub(crate) previous_head: Option<String>,
    pub(crate) git_snapshot: SourceBlob,
    pub(crate) fence: scope_postgres::db::ContentRefFence,
}

pub(crate) async fn persist_request_ref_to_store(
    state: &AppState,
    repo: &Repository,
    staging_repo: &FsPath,
    request: &Request,
    update: &RequestRefUpdate,
) -> Result<PersistedRequestRef, ApiError> {
    ensure_request_ref_oid_is_commit(staging_repo, &update.new_head_oid)?;
    ensure_request_ref_descends_from_base(
        staging_repo,
        &request.base_main_oid,
        &update.new_head_oid,
    )?;
    if request.audience == RequestAudience::Public {
        ensure_public_request_ref_is_public_safe(repo, state, staging_repo, &update.new_head_oid)
            .await?;
    }
    let incarnation = repo.incarnation();
    let _store_lock = acquire_request_ref_store_lock(state, &incarnation)?;
    let store_repo = ensure_request_ref_store_repo_locked(state, &incarnation)?;
    ensure_request_ref_available_in_store_locked(state, &store_repo, request)?;
    let previous_head = request_ref_head(&store_repo, &update.request_ref)?;
    let expected_stored_head = previous_head.as_deref().or_else(|| {
        request
            .git_snapshot
            .is_none()
            .then_some(request.head_oid.as_str())
    });
    let logical_old_head = update
        .old_head_oid
        .as_deref()
        .or(Some(request.head_oid.as_str()));
    validate_pushed_commit_range(staging_repo, logical_old_head, &update.new_head_oid)?;
    ensure_request_ref_store_head_matches_push(expected_stored_head, logical_old_head)?;
    ensure_request_ref_is_fast_forward(staging_repo, logical_old_head, &update.new_head_oid)?;
    let refspec = format!("+{}:{}", update.request_ref, update.request_ref);
    run_git(
        Some(&store_repo),
        &["fetch", staging_repo.to_string_lossy().as_ref(), &refspec],
        "persisting request ref",
    )?;
    let (git_snapshot, snapshot_bytes) =
        match git_snapshot_from_ref(&store_repo, &update.request_ref) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                rollback_request_ref(state, &incarnation, &update.request_ref, previous_head);
                return Err(error);
            }
        };
    let fence = match state
        .metadata
        .acquire_content_ref_fence(std::slice::from_ref(&git_snapshot.content_ref))
        .await
    {
        Ok(fence) => fence,
        Err(error) => {
            rollback_request_ref(state, &incarnation, &update.request_ref, previous_head);
            return Err(error.into());
        }
    };
    if let Err(error) = state.object_store.put(
        &scope_object_store::object_key(&git_snapshot),
        snapshot_bytes,
    ) {
        rollback_request_ref(state, &incarnation, &update.request_ref, previous_head);
        fence.release().await;
        return Err(error.into());
    }
    Ok(PersistedRequestRef {
        previous_head,
        git_snapshot,
        fence,
    })
}

fn ensure_request_ref_descends_from_base(
    repo: &FsPath,
    base_oid: &str,
    head_oid: &str,
) -> Result<(), ApiError> {
    let output = run_git_output(
        Some(repo),
        &["merge-base", "--is-ancestor", base_oid, head_oid],
        "checking request branch ancestry",
    )?;
    if output.status.success() {
        return Ok(());
    }
    Err(ApiError::conflict(
        "request branch must descend from its recorded base",
    ))
}

fn ensure_request_ref_is_fast_forward(
    repo: &FsPath,
    old_head_oid: Option<&str>,
    new_head_oid: &str,
) -> Result<(), ApiError> {
    let Some(old_head_oid) = old_head_oid else {
        return Ok(());
    };
    let output = run_git_output(
        Some(repo),
        &["merge-base", "--is-ancestor", old_head_oid, new_head_oid],
        "checking request branch fast-forward",
    )?;
    if output.status.success() {
        return Ok(());
    }
    Err(ApiError::conflict(
        "request branch update must be a fast-forward; fetch and rebase",
    ))
}

fn ensure_request_ref_oid_is_commit(repo: &FsPath, oid: &str) -> Result<(), ApiError> {
    if request_ref_oid_is_commit(repo, oid)? {
        return Ok(());
    }
    Err(ApiError::bad_request(
        "Scope request refs must point at commits",
    ))
}

fn request_ref_oid_is_commit(repo: &FsPath, oid: &str) -> Result<bool, ApiError> {
    let output = run_git_output(
        Some(repo),
        &["cat-file", "-t", oid],
        "validating request ref commit",
    )?;
    Ok(output.status.success()
        && String::from_utf8(output.stdout)
            .map_err(ApiError::bad_request)?
            .trim()
            == "commit")
}

fn ensure_request_ref_available_in_store_locked(
    state: &AppState,
    store_repo: &FsPath,
    request: &Request,
) -> Result<(), ApiError> {
    let request_ref = canonical_request_ref(&request.name);
    if request_ref_head(store_repo, &request_ref)?.as_deref() == Some(request.head_oid.as_str()) {
        return Ok(());
    }
    if let Some(snapshot) = request.git_snapshot.as_ref() {
        restore_request_ref_from_snapshot(state, store_repo, request, snapshot)?;
        if request_ref_head(store_repo, &request_ref)?.as_deref() == Some(request.head_oid.as_str())
        {
            return Ok(());
        }
        return Err(ApiError::infrastructure_unavailable(
            "stored request branch snapshot does not match request metadata",
        ));
    }
    if request_ref_exists(store_repo, &request_ref)? {
        run_git(
            Some(store_repo),
            &["update-ref", "-d", &request_ref],
            "deleting stale request ref cache",
        )?;
    }
    Ok(())
}

fn restore_request_ref_from_snapshot(
    state: &AppState,
    store_repo: &FsPath,
    request: &Request,
    snapshot: &SourceBlob,
) -> Result<(), ApiError> {
    let bundle_path = store_repo.with_extension(format!(
        "request-ref-{}.bundle.tmp",
        hex::encode(&snapshot.sha256.as_bytes()[..8])
    ));
    let bytes = source_blob_bytes(state.object_store.as_ref(), snapshot)?;
    fs::write(&bundle_path, bytes).map_err(ApiError::internal)?;
    let bundle = bundle_path.to_string_lossy().to_string();
    let request_ref = canonical_request_ref(&request.name);
    let refspec = format!("+{request_ref}:{request_ref}");
    let result = run_git(
        Some(store_repo),
        &["fetch", &bundle, &refspec],
        "restoring request ref snapshot",
    );
    let _ = fs::remove_file(&bundle_path);
    result
}

fn ensure_request_ref_store_head_matches_push(
    stored_head: Option<&str>,
    advertised_old_head: Option<&str>,
) -> Result<(), ApiError> {
    if stored_head == advertised_old_head {
        return Ok(());
    }
    Err(ApiError::conflict(
        "request branch changed since push started; fetch and retry",
    ))
}

fn ensure_request_ref_store_repo_locked(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
) -> Result<PathBuf, ApiError> {
    let store_repo = request_ref_store_repo_path(state, incarnation);
    if store_repo.join("objects").is_dir() {
        return Ok(store_repo);
    }
    if store_repo.exists() {
        remove_dir_if_exists(&store_repo)?;
    }
    if let Some(parent) = store_repo.parent() {
        crate::persistence::ensure_private_dir(parent)?;
    }
    run_git(
        None,
        &["init", "--bare", store_repo.to_string_lossy().as_ref()],
        "initializing request ref store",
    )?;
    run_git(
        Some(&store_repo),
        &[
            "symbolic-ref",
            "HEAD",
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "setting request ref store head",
    )?;
    Ok(store_repo)
}

fn request_ref_exists(store_repo: &FsPath, request_ref: &str) -> Result<bool, ApiError> {
    Ok(request_ref_head(store_repo, request_ref)?.is_some())
}

fn request_ref_head(store_repo: &FsPath, request_ref: &str) -> Result<Option<String>, ApiError> {
    if !store_repo.exists() {
        return Ok(None);
    }
    let output = run_git_output(
        Some(store_repo),
        &["rev-parse", "--verify", request_ref],
        "reading stored request ref",
    )?;
    if output.status.success() {
        let head = String::from_utf8(output.stdout).map_err(ApiError::bad_request)?;
        return Ok(Some(head.trim().to_string()));
    }
    Ok(None)
}

pub(crate) fn rollback_request_ref(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request_ref: &str,
    previous_head: Option<String>,
) {
    let store_repo = request_ref_store_repo_path(state, incarnation);
    let result = match previous_head {
        Some(head) => run_git(
            Some(&store_repo),
            &["update-ref", request_ref, &head],
            "rolling back request ref",
        ),
        None => {
            if store_repo.exists() {
                run_git(
                    Some(&store_repo),
                    &["update-ref", "-d", request_ref],
                    "deleting rolled-back request ref",
                )
            } else {
                Ok(())
            }
        }
    };
    if let Err(error) = result {
        tracing::warn!(
            repository_id = incarnation.repository_id(),
            repository_incarnation_id = incarnation.incarnation_id(),
            request_ref,
            error = error.operator_diagnostic(),
            "failed to roll back request ref after metadata rejection"
        );
    }
}

#[cfg(test)]
mod tests;
