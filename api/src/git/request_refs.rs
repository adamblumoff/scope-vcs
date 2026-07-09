use crate::{
    auth::scope::principal_for_user_id,
    config::{DEFAULT_GIT_BRANCH, EMPTY_GIT_OID},
    domain::{
        projection::{ProjectionViewKey, project_graph},
        requests::{
            REQUEST_REF_PREFIX, RecordRequestRevisionInput, RecordWorkingRequestUploadInput,
            Request, RequestActorRole, RequestBaseAudience, RequestState,
        },
        store::{RepoPublicationState, RepositoryActor, SourceBlob},
    },
    error::ApiError,
    git::{
        import::{
            git_snapshot_from_ref, run_git, run_git_output, safe_repo_key, validate_pushed_tree,
        },
        request_ref_public_safety::ensure_public_request_ref_is_public_safe,
        storage::{
            cached_raw_git_snapshot_repo, git_repo_storage_root, receive_pack_staging_repo_path,
            remove_dir_if_exists, request_ref_store_repo_path, write_receive_pack_hook,
        },
        upload::projection_bare_repo_for_state,
    },
    object_store::source_blob_bytes,
    persistence::unix_now,
    state::{AppState, find_repo},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path as FsPath, PathBuf},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

const REQUEST_REF_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_REF_LOCK_RETRY: Duration = Duration::from_millis(10);
const REQUEST_REF_STALE_LOCK_AFTER_SECS: u64 = 30 * 60;
static STALE_GIT_LOCK_REMOVAL: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestRefUpdate {
    pub(crate) request_ref: String,
    pub(crate) old_head_oid: Option<String>,
    pub(crate) new_head_oid: String,
}

pub(crate) fn is_request_ref(refname: &str) -> bool {
    refname
        .strip_prefix(REQUEST_REF_PREFIX)
        .is_some_and(|request_id| !request_id.trim().is_empty())
}

pub(crate) fn receive_pack_refs(staging_repo: &FsPath) -> Result<Vec<(String, String)>, ApiError> {
    refs_for_prefixes(
        staging_repo,
        &["refs/heads", "refs/tags", REQUEST_REF_PREFIX],
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
        if !is_request_ref(refname) {
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
        changed.push(RequestRefUpdate {
            request_ref: refname.clone(),
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
        .filter(|refname| !is_request_ref(refname))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .any(|refname| before.get(refname) != after.get(refname))
}

pub(crate) fn actor_has_open_editable_request(
    state: &AppState,
    repo_id: &str,
    actor_user_id: &str,
    current_actor: RepositoryActor,
) -> Result<bool, ApiError> {
    Ok(state
        .metadata
        .requests_by_repo_id(repo_id)?
        .into_iter()
        .any(|request| request_actor_can_edit_ref(&request, actor_user_id, current_actor)))
}

pub(crate) fn ensure_request_receive_pack_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
) -> Result<PathBuf, ApiError> {
    let repo = find_repo(state, owner, repo_name)?;
    if repo.record.publication_state != RepoPublicationState::Published {
        return Err(ApiError::not_found(format!(
            "repo {owner}/{repo_name} not found"
        )));
    }
    let access = repo.access_for_user_id(actor_user_id);
    if access.actor == RepositoryActor::Public
        && !actor_has_open_editable_request(state, &repo.record.id, actor_user_id, access.actor)?
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
            projection_bare_repo_for_state(state, &projection)?
        }
        RepositoryActor::Owner | RepositoryActor::Member => {
            if let Some(snapshot) = repo.git_snapshot.as_ref() {
                cached_raw_git_snapshot_repo(state, snapshot)?
            } else {
                let principal = principal_for_user_id(&repo, actor_user_id);
                let projection = project_graph(
                    &repo.policy,
                    &repo.graph,
                    &repo.visibility_events,
                    ProjectionViewKey::from_access(repo.access_for_principal(&principal)),
                );
                projection_bare_repo_for_state(state, &projection)?
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
    let setup_result = (|| {
        run_git(
            Some(&repo_root),
            &["config", "http.receivepack", "true"],
            "enabling request receive-pack",
        )?;
        seed_editable_request_refs(state, owner, repo_name, actor_user_id, &repo_root)?;
        install_request_pre_receive_hook(&repo_root)
    })();
    if let Err(error) = setup_result {
        let _ = fs::remove_dir_all(&repo_root);
        return Err(error);
    }
    Ok(repo_root)
}

pub(crate) fn seed_editable_request_refs(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    staging_repo: &FsPath,
) -> Result<(), ApiError> {
    let repo = find_repo(state, owner, repo_name)?;
    let access = repo.access_for_user_id(actor_user_id);
    let requests = state
        .metadata
        .requests_by_repo_id(&repo.record.id)?
        .into_iter()
        .filter(|request| request_actor_can_edit_ref(request, actor_user_id, access.actor))
        .collect::<Vec<_>>();
    let has_durable_request_ref = requests
        .iter()
        .any(|request| request.git_snapshot.is_some());
    let store_repo_path = request_ref_store_repo_path(state, owner, repo_name);
    if !store_repo_path.exists() && !has_durable_request_ref {
        return Ok(());
    }

    for request in requests {
        let _update_lock =
            acquire_request_ref_update_lock(state, owner, repo_name, &request.request_ref)?;
        let Some(request) = state.metadata.request_by_ref(&request.request_ref)? else {
            continue;
        };
        if request.repo_id != repo.record.id
            || !request_actor_can_edit_ref(&request, actor_user_id, access.actor)
        {
            continue;
        }
        let _store_lock = acquire_request_ref_store_lock(state, owner, repo_name)?;
        let store_repo = ensure_request_ref_store_repo_locked(state, owner, repo_name)?;
        ensure_request_ref_available_in_store_locked(state, &store_repo, &request)?;
        let refspec = format!("+{}:{}", request.request_ref, request.request_ref);
        let output = run_git_output(
            Some(staging_repo),
            &["fetch", store_repo.to_string_lossy().as_ref(), &refspec],
            "fetching stored request ref",
        )?;
        if !output.status.success() && request_ref_exists(&store_repo, &request.request_ref)? {
            return Err(ApiError::service_unavailable(format!(
                "fetching stored request ref: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    }
    Ok(())
}

pub(crate) fn persist_request_ref_revision(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    staging_repo: &FsPath,
    update: RequestRefUpdate,
) -> Result<(), ApiError> {
    let _lock = acquire_request_ref_update_lock(state, owner, repo_name, &update.request_ref)?;
    let request =
        ensure_request_ref_update_allowed(state, owner, repo_name, actor_user_id, &update)?;
    let now_unix = unix_now()?;
    let persisted =
        persist_request_ref_to_store(state, owner, repo_name, staging_repo, &request, &update)?;
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
            .map(|mutation| mutation.source_blobs_to_delete)
    } else {
        state
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
            .map(|mutation| mutation.source_blobs_to_delete)
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
        );
        return Err(error);
    }
    crate::state::best_effort_drain_pending_source_blob_deletions(state);
    Ok(())
}

pub(crate) fn request_ref_bundle_bytes(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    request: &Request,
) -> Result<Vec<u8>, ApiError> {
    with_request_ref_store_repo(state, owner, repo_name, request, |store_repo| {
        let mut hasher = Sha256::new();
        hasher.update(request.request_ref.as_bytes());
        hasher.update(request.head_oid.as_bytes());
        let bundle_path = store_repo.with_extension(format!(
            "request-{}.bundle.tmp",
            hex::encode(&hasher.finalize()[..8])
        ));
        let result = (|| {
            let output = run_git_output(
                Some(store_repo),
                &[
                    "bundle",
                    "create",
                    bundle_path.to_string_lossy().as_ref(),
                    &request.request_ref,
                ],
                "creating request branch bundle",
            )?;
            if !output.status.success() {
                return Err(ApiError::service_unavailable(format!(
                    "creating request branch bundle: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
            fs::read(&bundle_path).map_err(ApiError::internal)
        })();
        let _ = fs::remove_file(&bundle_path);
        result
    })
}

pub(crate) fn with_request_ref_store_repo<T>(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    request: &Request,
    action: impl FnOnce(&FsPath) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    if request.git_snapshot.is_none() {
        return Err(ApiError::conflict("request branch has not been pushed"));
    }
    let _update_lock =
        acquire_request_ref_update_lock(state, owner, repo_name, &request.request_ref)?;
    let _store_lock = acquire_request_ref_store_lock(state, owner, repo_name)?;
    let store_repo = ensure_request_ref_store_repo_locked(state, owner, repo_name)?;
    ensure_request_ref_available_in_store_locked(state, &store_repo, request)?;
    action(&store_repo)
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

fn request_is_closed(request: &Request) -> bool {
    matches!(
        request.state,
        RequestState::Resolved | RequestState::Withdrawn
    )
}

fn request_is_open_for_current_actor(request: &Request, current_actor: RepositoryActor) -> bool {
    if request_is_closed(request) {
        return false;
    }
    match current_actor {
        RepositoryActor::Public => {
            request.author_role == RequestActorRole::Public
                && request.base_audience == RequestBaseAudience::Public
        }
        RepositoryActor::Member | RepositoryActor::Owner => true,
    }
}

fn request_actor_can_edit_ref(
    request: &Request,
    actor_user_id: &str,
    current_actor: RepositoryActor,
) -> bool {
    if !request_is_open_for_current_actor(request, current_actor) {
        return false;
    }
    match current_actor {
        RepositoryActor::Public => {
            request.author_user_id == actor_user_id
                || request.editor_user_ids.contains(actor_user_id)
        }
        RepositoryActor::Member | RepositoryActor::Owner => true,
    }
}

fn install_request_pre_receive_hook(repo_root: &FsPath) -> Result<(), ApiError> {
    let hook = repo_root.join("hooks").join("pre-receive");
    let script = format!(
        "#!/bin/sh\ncount=0\nwhile read old new ref; do\n  count=$((count + 1))\n  case \"$ref\" in\n    refs/scope/requests/*) ;;\n    *)\n      echo \"Scope request pushes only accept refs/scope/requests/*\" >&2\n      exit 1\n      ;;\n  esac\n  if [ \"$new\" = \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope does not accept request branch deletes\" >&2\n    exit 1\n  fi\n  if [ \"$(git cat-file -t \"$new\" 2>/dev/null)\" != \"commit\" ]; then\n    echo \"Scope request refs must point at commits\" >&2\n    exit 1\n  fi\ndone\nif [ \"$count\" -ne 1 ]; then\n  echo \"Scope accepts exactly one request ref update\" >&2\n  exit 1\nfi\n"
    );
    write_receive_pack_hook(&hook, &script)
}

fn ensure_request_ref_update_allowed(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    update: &RequestRefUpdate,
) -> Result<Request, ApiError> {
    let repo = find_repo(state, owner, repo_name)?;
    let request = state
        .metadata
        .request_by_ref(&update.request_ref)?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if request.repo_id != repo.record.id {
        return Err(ApiError::not_found("request not found"));
    }
    let current_actor = repo.access_for_user_id(actor_user_id).actor;
    if !request_is_open_for_current_actor(&request, current_actor) {
        if request_is_closed(&request)
            && (request.author_user_id == actor_user_id
                || request.editor_user_ids.contains(actor_user_id))
        {
            return Err(ApiError::conflict("request is closed"));
        }
        return Err(ApiError::not_found("request not found"));
    }
    if !request_actor_can_edit_ref(&request, actor_user_id, current_actor) {
        return Err(ApiError::not_found("request not found"));
    }
    Ok(request)
}

struct PersistedRequestRef {
    previous_head: Option<String>,
    git_snapshot: SourceBlob,
}

fn persist_request_ref_to_store(
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
    let repo = find_repo(state, owner, repo_name)?;
    if request.base_audience == RequestBaseAudience::Public {
        ensure_public_request_ref_is_public_safe(&repo, state, staging_repo, &update.new_head_oid)?;
    }
    let _store_lock = acquire_request_ref_store_lock(state, owner, repo_name)?;
    let store_repo = ensure_request_ref_store_repo_locked(state, owner, repo_name)?;
    let previous_head = request_ref_head(&store_repo, &update.request_ref)?;
    ensure_request_ref_store_head_matches_push(
        previous_head.as_deref(),
        update.old_head_oid.as_deref(),
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

fn ensure_request_ref_oid_is_commit(repo: &FsPath, oid: &str) -> Result<(), ApiError> {
    let output = run_git_output(
        Some(repo),
        &["cat-file", "-t", oid],
        "validating request ref commit",
    )?;
    if output.status.success()
        && String::from_utf8(output.stdout)
            .map_err(ApiError::bad_request)?
            .trim()
            == "commit"
    {
        return Ok(());
    }
    Err(ApiError::bad_request(
        "Scope request refs must point at commits",
    ))
}

fn ensure_request_ref_available_in_store_locked(
    state: &AppState,
    store_repo: &FsPath,
    request: &Request,
) -> Result<(), ApiError> {
    if request_ref_head(store_repo, &request.request_ref)?.as_deref()
        == Some(request.head_oid.as_str())
    {
        return Ok(());
    }
    if let Some(snapshot) = request.git_snapshot.as_ref() {
        restore_request_ref_from_snapshot(state, store_repo, request, snapshot)?;
        if request_ref_head(store_repo, &request.request_ref)?.as_deref()
            == Some(request.head_oid.as_str())
        {
            return Ok(());
        }
        return Err(ApiError::service_unavailable(
            "stored request branch snapshot does not match request metadata",
        ));
    }
    if request_ref_exists(store_repo, &request.request_ref)? {
        run_git(
            Some(store_repo),
            &["update-ref", "-d", &request.request_ref],
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
    let refspec = format!("+{}:{}", request.request_ref, request.request_ref);
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
            error = %error.message,
            "failed to roll back request ref after metadata rejection"
        );
    }
}

struct GitLockFile {
    path: PathBuf,
}

impl Drop for GitLockFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn acquire_request_ref_update_lock(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    request_ref: &str,
) -> Result<GitLockFile, ApiError> {
    let path = request_ref_update_lock_path(state, owner, repo_name, request_ref);
    acquire_git_lock(path, "request branch update already in progress")
}

fn acquire_request_ref_store_lock(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<GitLockFile, ApiError> {
    let path = request_ref_store_lock_path(state, owner, repo_name);
    acquire_git_lock(
        path,
        "request branch store initialization already in progress",
    )
}

fn acquire_git_lock(
    path: PathBuf,
    conflict_message: &'static str,
) -> Result<GitLockFile, ApiError> {
    acquire_git_lock_with_stale_cleanup(path, conflict_message, true)
}

fn acquire_git_lock_with_stale_cleanup(
    path: PathBuf,
    conflict_message: &'static str,
    stale_cleanup: bool,
) -> Result<GitLockFile, ApiError> {
    if let Some(parent) = path.parent() {
        crate::persistence::ensure_private_dir(parent)?;
    }
    let started_at = Instant::now();
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = writeln!(
                    file,
                    "pid={}\ncreated_at_unix={}",
                    std::process::id(),
                    unix_now()?
                ) {
                    let _ = fs::remove_file(&path);
                    return Err(ApiError::internal(error));
                }
                return Ok(GitLockFile { path });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                if stale_cleanup && remove_stale_git_lock(&path)? {
                    continue;
                }
                if started_at.elapsed() >= REQUEST_REF_LOCK_TIMEOUT {
                    return Err(ApiError::conflict(conflict_message));
                }
                thread::sleep(REQUEST_REF_LOCK_RETRY);
            }
            Err(error) => return Err(ApiError::internal(error)),
        }
    }
}

fn remove_stale_git_lock(path: &FsPath) -> Result<bool, ApiError> {
    let _guard = STALE_GIT_LOCK_REMOVAL.lock().map_err(|_| {
        ApiError::internal(std::io::Error::other(
            "stale git lock removal mutex poisoned",
        ))
    })?;
    let recovery_path = stale_git_lock_recovery_path(path);
    let _recovery_lock = acquire_git_lock_with_stale_cleanup(
        recovery_path,
        "request branch lock recovery already in progress",
        false,
    )?;
    if !git_lock_is_stale(path)? {
        return Ok(false);
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(true),
        Err(error) => Err(ApiError::internal(error)),
    }
}

fn stale_git_lock_recovery_path(path: &FsPath) -> PathBuf {
    let mut recovery_path = path.to_path_buf();
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    recovery_path.set_file_name(format!("{file_name}.recovery"));
    recovery_path
}

fn git_lock_is_stale(path: &FsPath) -> Result<bool, ApiError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(true),
        Err(_) => String::new(),
    };
    let pid = lock_field(&text, "pid").and_then(|value| value.parse::<u32>().ok());
    if let Some(pid) = pid
        && !process_is_alive(pid)
    {
        return Ok(true);
    }
    let created_at_unix =
        lock_field(&text, "created_at_unix").and_then(|value| value.parse::<u64>().ok());
    if let Some(created_at_unix) = created_at_unix {
        return Ok(unix_now()?.saturating_sub(created_at_unix) >= REQUEST_REF_STALE_LOCK_AFTER_SECS);
    }
    let modified_at = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(ApiError::internal)?;
    Ok(modified_at
        .elapsed()
        .map(|elapsed| elapsed.as_secs() >= REQUEST_REF_STALE_LOCK_AFTER_SECS)
        .unwrap_or(false))
}

fn lock_field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}=");
    text.lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
}

#[cfg(target_os = "linux")]
fn process_is_alive(pid: u32) -> bool {
    PathBuf::from(format!("/proc/{pid}")).exists()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_is_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> bool {
    true
}

fn request_ref_store_lock_path(state: &AppState, owner: &str, repo_name: &str) -> PathBuf {
    let repo_key = safe_repo_key(owner, repo_name);
    git_repo_storage_root(state)
        .join("git-request-refs-locks")
        .join(format!("{repo_key}-store.lock"))
}

fn request_ref_update_lock_path(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    request_ref: &str,
) -> PathBuf {
    let repo_key = safe_repo_key(owner, repo_name);
    let ref_hash = hex::encode(Sha256::digest(request_ref.as_bytes()));
    git_repo_storage_root(state)
        .join("git-request-refs-locks")
        .join(format!("{repo_key}-{ref_hash}.lock"))
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
