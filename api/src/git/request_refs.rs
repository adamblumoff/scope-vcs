use crate::{
    auth::scope::principal_for_user_id,
    config::{DEFAULT_GIT_BRANCH, EMPTY_GIT_OID},
    domain::{
        projection::{ProjectionViewKey, project_graph},
        requests::{
            RecordRequestRevisionInput, RecordWorkingRequestUploadInput, Request, RequestAudience,
            RequestChangeBlock, RequestState, canonical_request_ref,
        },
        store::{RepoPublicationState, RepositoryActor, SourceBlob},
    },
    error::ApiError,
    git::{
        cache::GitRepoHandle,
        import::{git_snapshot_from_ref, run_git, run_git_output, validate_pushed_tree},
        request_ref_public_safety::ensure_public_request_ref_is_public_safe,
        storage::{
            cached_raw_git_repo, receive_pack_staging_repo_path, remove_dir_if_exists,
            request_ref_store_repo_path, write_receive_pack_hook,
        },
        upload::projection_bare_repo_for_state,
    },
    object_store::source_blob_bytes,
    persistence::unix_now,
    state::{AppState, find_repo},
};
use access::{ensure_request_ref_update_allowed, request_actor_can_edit_ref};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path as FsPath, PathBuf},
};

mod access;
mod locks;
#[cfg(test)]
use locks::git_lock_is_stale;
use locks::{acquire_request_ref_store_lock, acquire_request_ref_update_lock};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestRefUpdate {
    pub(crate) request_ref: String,
    pub(crate) request_name: String,
    pub(crate) old_head_oid: Option<String>,
    pub(crate) new_head_oid: String,
}

pub(crate) fn is_request_ref(refname: &str) -> bool {
    request_name_from_ref(refname)
        .is_some_and(|name| crate::domain::requests::validate_request_name(name).is_ok())
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
            crate::domain::requests::validate_request_name(request_name).map_err(|error| {
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

pub(crate) async fn actor_has_open_editable_request(
    state: &AppState,
    repo_id: &str,
    actor_user_id: &str,
    access: crate::domain::store::RepositoryAccess,
) -> Result<bool, ApiError> {
    Ok(state
        .metadata
        .requests_by_repo_id(repo_id)
        .await?
        .into_iter()
        .any(|request| request_actor_can_edit_ref(&request, actor_user_id, access)))
}

pub(crate) async fn ensure_request_receive_pack_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
) -> Result<PathBuf, ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    if repo.record.publication_state != RepoPublicationState::Published {
        return Err(ApiError::not_found(format!(
            "repo {owner}/{repo_name} not found"
        )));
    }
    let access = repo.access_for_user_id(actor_user_id);
    if access.actor == RepositoryActor::Public
        && !actor_has_open_editable_request(state, &repo.record.id, actor_user_id, access).await?
    {
        return Err(ApiError::not_found(format!(
            "repo {owner}/{repo_name} not found"
        )));
    }

    let seed_repo = match access.actor {
        RepositoryActor::Public => {
            let projection = project_graph(
                &repo.policy,
                &repo.graph,
                &repo.visibility_events,
                ProjectionViewKey::Public,
            );
            GitRepoHandle::from_path(projection_bare_repo_for_state(state, &projection)?)
        }
        RepositoryActor::Owner | RepositoryActor::Member => {
            if let Some(head) = repo.git_head.as_ref() {
                cached_raw_git_repo(state, &head.manifest)?
            } else {
                let principal = principal_for_user_id(&repo, actor_user_id);
                let projection = project_graph(
                    &repo.policy,
                    &repo.graph,
                    &repo.visibility_events,
                    ProjectionViewKey::from_access(repo.access_for_principal(&principal)),
                );
                GitRepoHandle::from_path(projection_bare_repo_for_state(state, &projection)?)
            }
        }
    };

    let repo_root = receive_pack_staging_repo_path(state, owner, repo_name)?;
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
    let setup_result = async {
        run_git(
            Some(&repo_root),
            &["config", "http.receivepack", "true"],
            "enabling request receive-pack",
        )?;
        seed_editable_request_refs(state, owner, repo_name, actor_user_id, &repo_root).await?;
        install_request_pre_receive_hook(&repo_root)
    }
    .await;
    if let Err(error) = setup_result {
        let _ = fs::remove_dir_all(&repo_root);
        return Err(error);
    }
    Ok(repo_root)
}

pub(crate) async fn seed_editable_request_refs(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    staging_repo: &FsPath,
) -> Result<(), ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    let access = repo.access_for_user_id(actor_user_id);
    let requests = state
        .metadata
        .requests_by_repo_id(&repo.record.id)
        .await?
        .into_iter()
        .filter(|request| request_actor_can_edit_ref(request, actor_user_id, access))
        .collect::<Vec<_>>();
    let public_base_repo = if access.actor != RepositoryActor::Public
        && requests.iter().any(|request| {
            request.audience == RequestAudience::Public && request.git_snapshot.is_none()
        }) {
        let projection = project_graph(
            &repo.policy,
            &repo.graph,
            &repo.visibility_events,
            ProjectionViewKey::Public,
        );
        Some(projection_bare_repo_for_state(state, &projection)?)
    } else {
        None
    };
    attach_visible_request_refs(state, &requests, staging_repo, public_base_repo.as_deref())
}

pub(crate) async fn persist_request_ref_revision(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    staging_repo: &FsPath,
    update: RequestRefUpdate,
) -> Result<(), ApiError> {
    let _lock = acquire_request_ref_update_lock(state, owner, repo_name, &update.request_ref)?;
    let request = ensure_request_ref_update_allowed(
        state,
        owner,
        repo_name,
        actor_user_id,
        &update.request_name,
    )
    .await?;
    let request_id = request.id.clone();
    let request_repo_id = request.repo_id.clone();
    let request_audience = request.audience;
    let now_unix = unix_now()?;
    let persisted =
        persist_request_ref_to_store(state, owner, repo_name, staging_repo, &request, &update)
            .await?;
    let mutation_result = if request.state == RequestState::Working {
        state
            .metadata
            .record_working_request_upload(RecordWorkingRequestUploadInput {
                request_id: request.id,
                actor_user_id: actor_user_id.to_string(),
                actor_can_edit: false,
                expected_old_head_oid: update.old_head_oid.clone(),
                new_head_oid: update.new_head_oid.clone(),
                git_snapshot: persisted.git_snapshot.clone(),
                now_unix,
            })
            .await
            .map(|mutation| mutation.orphan_objects)
    } else {
        let mutation = state
            .metadata
            .record_request_revision(RecordRequestRevisionInput {
                request_id: request.id,
                actor_user_id: actor_user_id.to_string(),
                actor_can_edit: false,
                expected_old_head_oid: update.old_head_oid.clone(),
                new_head_oid: update.new_head_oid.clone(),
                git_snapshot: Some(persisted.git_snapshot.clone()),
                event_id: request_revision_event_id()?,
                body: None,
                now_unix,
            })
            .await;
        if let Ok(mutation) = &mutation {
            state
                .publish_request_timeline_change(
                    &request_repo_id,
                    request_id,
                    mutation.discussion.id.clone(),
                    mutation.discussion.last_activity_position,
                    request_audience,
                )
                .await;
        }
        mutation.map(|_| Vec::new())
    };
    if let Err(error) = mutation_result {
        rollback_request_ref(
            state,
            owner,
            repo_name,
            &update.request_ref,
            persisted.previous_head,
        );
        crate::state::best_effort_cleanup_rollback_source_blobs(
            state,
            std::slice::from_ref(&persisted.git_snapshot),
        )
        .await;
        return Err(error.into());
    }
    Ok(())
}

pub(crate) fn with_request_change_block_store_repo<T>(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    request: &Request,
    block: &RequestChangeBlock,
    action: impl FnOnce(&FsPath) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    let request_ref = canonical_request_ref(&request.name);
    let _update_lock = acquire_request_ref_update_lock(state, owner, repo_name, &request_ref)?;
    let _store_lock = acquire_request_ref_store_lock(state, owner, repo_name)?;
    let store_repo = ensure_request_ref_store_repo_locked(state, owner, repo_name)?;
    let bundle_path = store_repo.with_extension(format!("change-block-{}.bundle.tmp", block.id));
    let temporary_ref = format!("refs/scope/internal/change-block/{}", block.id);
    let bytes = source_blob_bytes(state.object_store.as_ref(), &block.git_snapshot)?;
    fs::write(&bundle_path, bytes).map_err(ApiError::internal)?;
    let bundle = bundle_path.to_string_lossy().to_string();
    let refspec = format!("+{request_ref}:{temporary_ref}");
    let imported = run_git(
        Some(&store_repo),
        &["fetch", &bundle, &refspec],
        "attaching request change block",
    );
    let _ = fs::remove_file(&bundle_path);
    imported?;
    if !request_ref_oid_is_commit(&store_repo, &block.new_head_oid)? {
        let _ = run_git(
            Some(&store_repo),
            &["update-ref", "-d", &temporary_ref],
            "removing request change block ref",
        );
        return Err(ApiError::service_unavailable(
            "request change block snapshot does not contain its head",
        ));
    }
    let result = action(&store_repo);
    let cleanup = run_git(
        Some(&store_repo),
        &["update-ref", "-d", &temporary_ref],
        "removing request change block ref",
    );
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
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
            return Err(ApiError::service_unavailable(
                "request snapshot does not match request metadata",
            ));
        }
    }
    Ok(())
}

pub(crate) fn delete_request_ref_from_store(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    request_ref: &str,
) -> Result<(), ApiError> {
    let _update_lock = acquire_request_ref_update_lock(state, owner, repo_name, request_ref)?;
    let store_repo = request_ref_store_repo_path(state, owner, repo_name);
    if !store_repo.exists() {
        return Ok(());
    }
    let _store_lock = acquire_request_ref_store_lock(state, owner, repo_name)?;
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
        return Err(ApiError::service_unavailable(format!(
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
        "#!/bin/sh\ncount=0\nwhile read old new ref; do\n  count=$((count + 1))\n  case \"$ref\" in\n    refs/heads/{DEFAULT_GIT_BRANCH})\n      echo \"Scope contributors cannot update main\" >&2\n      exit 1\n      ;;\n    refs/heads/*) ;;\n    *)\n      echo \"Scope request pushes only accept named request branches\" >&2\n      exit 1\n      ;;\n  esac\n  if [ \"$new\" = \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope does not accept request branch deletes\" >&2\n    exit 1\n  fi\n  if [ \"$old\" = \"{EMPTY_GIT_OID}\" ]; then\n    echo \"request not found; fetch before pushing\" >&2\n    exit 1\n  fi\n  if [ \"$(git cat-file -t \"$new\" 2>/dev/null)\" != \"commit\" ]; then\n    echo \"Scope request refs must point at commits\" >&2\n    exit 1\n  fi\n  if ! git merge-base --is-ancestor \"$old\" \"$new\"; then\n    echo \"Scope rejects non-fast-forward request pushes\" >&2\n    exit 1\n  fi\ndone\nif [ \"$count\" -ne 1 ]; then\n  echo \"Scope accepts exactly one request ref update\" >&2\n  exit 1\nfi\n"
    );
    write_receive_pack_hook(&hook, &script)
}

struct PersistedRequestRef {
    previous_head: Option<String>,
    git_snapshot: SourceBlob,
}

async fn persist_request_ref_to_store(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &FsPath,
    request: &Request,
    update: &RequestRefUpdate,
) -> Result<PersistedRequestRef, ApiError> {
    ensure_request_ref_oid_is_commit(staging_repo, &update.new_head_oid)?;
    validate_pushed_tree(staging_repo, &update.new_head_oid)?;
    ensure_request_ref_descends_from_base(
        staging_repo,
        &request.base_main_oid,
        &update.new_head_oid,
    )?;
    let repo = find_repo(state, owner, repo_name).await?;
    if request.audience == RequestAudience::Public {
        ensure_public_request_ref_is_public_safe(&repo, state, staging_repo, &update.new_head_oid)?;
    }
    let _store_lock = acquire_request_ref_store_lock(state, owner, repo_name)?;
    let store_repo = ensure_request_ref_store_repo_locked(state, owner, repo_name)?;
    ensure_request_ref_available_in_store_locked(state, &store_repo, request)?;
    let previous_head = request_ref_head(&store_repo, &update.request_ref)?;
    let expected_stored_head = previous_head.as_deref().or_else(|| {
        request
            .git_snapshot
            .is_none()
            .then_some(request.head_oid.as_str())
    });
    ensure_request_ref_store_head_matches_push(
        expected_stored_head,
        update.old_head_oid.as_deref(),
    )?;
    ensure_request_ref_is_fast_forward(
        staging_repo,
        update.old_head_oid.as_deref(),
        &update.new_head_oid,
    )?;
    let refspec = format!("+{}:{}", update.request_ref, update.request_ref);
    run_git(
        Some(&store_repo),
        &["fetch", staging_repo.to_string_lossy().as_ref(), &refspec],
        "persisting request ref",
    )?;
    let git_snapshot =
        match git_snapshot_from_ref(state, &request.repo_id, &store_repo, &update.request_ref) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                rollback_request_ref(state, owner, repo_name, &update.request_ref, previous_head);
                return Err(error);
            }
        };
    Ok(PersistedRequestRef {
        previous_head,
        git_snapshot,
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
        return Err(ApiError::service_unavailable(
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
    owner: &str,
    repo_name: &str,
) -> Result<PathBuf, ApiError> {
    let store_repo = request_ref_store_repo_path(state, owner, repo_name);
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

fn rollback_request_ref(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    request_ref: &str,
    previous_head: Option<String>,
) {
    let store_repo = request_ref_store_repo_path(state, owner, repo_name);
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
            owner,
            repo = repo_name,
            request_ref,
            error = error.message(),
            "failed to roll back request ref after metadata rejection"
        );
    }
}

fn request_revision_event_id() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to create request revision event id: {error}"
        ))
    })?;
    Ok(format!("event_request_revision_{}", hex::encode(bytes)))
}

#[cfg(test)]
mod tests;
