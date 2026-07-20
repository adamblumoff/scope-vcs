use super::env::DevSeedUser;
mod request_discussions;
pub(super) use request_discussions::seed_request_discussion_gallery;
#[cfg(test)]
use request_discussions::{
    CONTRIBUTOR_ID as DEV_SEED_CONTRIBUTOR_ID, MAINTAINER_ID as DEV_SEED_MAINTAINER_ID,
    REQUEST_ID as DISCUSSION_REQUEST_ID,
};

use crate::{
    config::DEFAULT_GIT_BRANCH,
    domain::{
        policy::{ScopePath, Visibility, VisibilityRule},
        projection::{AuthorVisibility, FileChange, LogicalCommit},
        requests::{
            DeleteRequestInput, MarkRequestNeedsResponseInput, MergeRequestInput,
            RecordWorkingRequestUploadInput, RequestActorRole, RequestAudience, StartRequestInput,
            SubmitRequestInput, canonical_request_ref, delete_request, mark_request_needs_response,
            merge_request, record_working_request_upload, start_request, submit_request,
        },
        store::{
            AppCatalog, GitHead, GitSegment, RepoPublicationState, SourceBlob, StoredRepository,
            UserAccount,
        },
    },
    error::ApiError,
    object_store::{ObjectStore, put_repo_object, put_source_blob},
};
use std::{
    fs,
    io::Write,
    path::Path as FsPath,
    process::{Command, Stdio},
};

pub(super) const DEV_SEED_USER_ID: &str = "scope_usr_dev_seed";
const PUBLIC_DEMO_README: &str =
    "# Public Demo\n\nThis seeded repository is ready to browse locally.\n";
const PUBLIC_DEMO_APP: &str =
    "export function greet(name: string) {\n  return `hello ${name}`\n}\n";
const PUBLIC_DEMO_PLAN: &str =
    "# Internal Plan\n\nPrivate content stays out of public projections.\n";
const UPDATE_DEMO_INITIAL_README: &str =
    "# Update Demo\n\nThis repository has a clean published baseline.\n";
const UPDATE_DEMO_RELEASE_GUIDE: &str =
    "# Release flow\n\nDocument the release before publishing the next version.\n";
const UPDATE_DEMO_RETRY_HELPER: &str =
    "export function retryDelay(attempt: number) {\n  return Math.min(attempt * 250, 2000)\n}\n";
const UPDATE_DEMO_TROUBLESHOOTING: &str =
    "# Troubleshooting\n\nExplain how to recover when the remote is unavailable.\n";
const UPDATE_DEMO_CLI_EXPERIMENT: &str =
    "experimental output: checking repository state before push\n";

pub(super) fn catalog(
    object_store: &dyn ObjectStore,
    seed_user: DevSeedUser,
) -> Result<AppCatalog, ApiError> {
    let owner = seed_user_account(seed_user);
    let [contributor, maintainer] = request_discussions::collaborators();
    let mut catalog = AppCatalog::default();
    catalog.users.insert(owner.id.clone(), owner.clone());
    catalog
        .users
        .insert(contributor.id.clone(), contributor.clone());
    catalog
        .users
        .insert(maintainer.id.clone(), maintainer.clone());

    let (mut update_demo, request_gallery) = update_demo(object_store, &owner)?;
    request_discussions::add_maintainer(&mut update_demo);
    for repo in [published_demo(object_store, &owner)?, update_demo] {
        catalog.repositories.insert(repo.record.id.clone(), repo);
    }
    seed_request_gallery(&mut catalog, &owner, request_gallery)?;

    Ok(catalog)
}

pub(super) fn seed_user_account(seed_user: DevSeedUser) -> UserAccount {
    UserAccount {
        id: DEV_SEED_USER_ID.to_string(),
        handle: seed_user.handle,
        email: seed_user.email,
        email_verified: true,
    }
}

fn published_demo(
    object_store: &dyn ObjectStore,
    owner: &UserAccount,
) -> Result<StoredRepository, ApiError> {
    let mut repo = repo(owner, "public-demo", Visibility::Public)?;
    let readme = blob(object_store, &repo, PUBLIC_DEMO_README)?;
    let app = blob(object_store, &repo, PUBLIC_DEMO_APP)?;
    let private_plan = blob(object_store, &repo, PUBLIC_DEMO_PLAN)?;
    let private_path = ScopePath::parse("/internal/plan.md").map_err(ApiError::internal)?;
    repo.policy
        .add_rule(VisibilityRule::private(private_path.clone()))
        .map_err(ApiError::internal)?;
    repo.graph.commits.push(commit(
        &repo,
        "dev-public-1",
        [],
        "Seed public demo",
        vec![
            add_change("/README.md", readme, Visibility::Public)?,
            add_change("/src/app.ts", app, Visibility::Public)?,
            add_change(private_path.as_str(), private_plan, Visibility::Private)?,
        ],
    ));
    populate_seed_live_files(&mut repo);
    repo.record.publication_state = RepoPublicationState::Published;
    let (head, segment) = git_segment_state(
        object_store,
        &repo,
        "public-demo-live",
        &[SeedGitCommit {
            files: &[
                ("README.md", PUBLIC_DEMO_README),
                ("src/app.ts", PUBLIC_DEMO_APP),
                ("internal/plan.md", PUBLIC_DEMO_PLAN),
            ],
            message: "Seed public demo",
        }],
    )?;
    repo.git_head = Some(head);
    repo.git_segments.push(segment);
    Ok(repo)
}

fn update_demo(
    object_store: &dyn ObjectStore,
    owner: &UserAccount,
) -> Result<(StoredRepository, SeedRequestGallery), ApiError> {
    let mut repo = repo(owner, "update-demo", Visibility::Public)?;
    let initial_readme = blob(object_store, &repo, UPDATE_DEMO_INITIAL_README)?;
    let release_guide = blob(object_store, &repo, UPDATE_DEMO_RELEASE_GUIDE)?;
    repo.graph.commits.push(commit(
        &repo,
        "dev-update-1",
        [],
        "Seed update demo",
        vec![add_change(
            "/README.md",
            initial_readme.clone(),
            Visibility::Public,
        )?],
    ));
    repo.graph.commits.push(commit(
        &repo,
        "dev-update-2",
        ["dev-update-1"],
        "Document release flow",
        vec![add_change(
            "/docs/release.md",
            release_guide,
            Visibility::Public,
        )?],
    ));
    populate_seed_live_files(&mut repo);
    repo.record.publication_state = RepoPublicationState::Published;
    let initial = SeedGitCommit {
        files: &[("README.md", UPDATE_DEMO_INITIAL_README)],
        message: "Seed update demo",
    };
    let accepted = SeedGitCommit {
        files: &[("docs/release.md", UPDATE_DEMO_RELEASE_GUIDE)],
        message: "Document release flow",
    };
    let (head, segment, gallery) =
        update_demo_git_snapshot(object_store, &repo, initial, accepted)?;
    repo.git_head = Some(head);
    repo.git_segments.push(segment);
    Ok((repo, gallery))
}

fn populate_seed_live_files(repo: &mut StoredRepository) {
    repo.live_files.clear();
    for change in repo.graph.commits.iter().flat_map(|commit| &commit.changes) {
        match &change.new_content {
            Some(content) => {
                repo.live_files.insert(change.path.clone(), content.clone());
            }
            None => {
                repo.live_files.remove(&change.path);
            }
        }
    }
}

type SeedRequestGallery = Vec<SeedRequest>;

struct SeedRequest {
    id: &'static str,
    name: &'static str,
    title: &'static str,
    base_oid: String,
    head_oid: String,
    snapshot: SourceBlob,
    outcome: SeedRequestOutcome,
    audience: RequestAudience,
    now_unix: u64,
}

enum SeedRequestOutcome {
    Submitted,
    NeedsResponse,
    Accepted,
    Withdrawn,
}

fn seed_request_gallery(
    catalog: &mut AppCatalog,
    owner: &UserAccount,
    gallery: SeedRequestGallery,
) -> Result<(), ApiError> {
    let repo_id = catalog
        .repository(&owner.handle, "update-demo")
        .ok_or_else(|| ApiError::internal_message("seeded update demo is missing"))?
        .record
        .id
        .clone();
    for request in gallery {
        seed_owner_request(catalog, owner, &repo_id, request)?;
    }
    Ok(())
}

fn seed_owner_request(
    catalog: &mut AppCatalog,
    owner: &UserAccount,
    repo_id: &str,
    request: SeedRequest,
) -> Result<(), ApiError> {
    let SeedRequest {
        id,
        name,
        title,
        base_oid,
        head_oid,
        snapshot,
        outcome,
        audience,
        now_unix,
    } = request;
    let started = start_request(
        &mut catalog.requests,
        StartRequestInput {
            id: id.to_string(),
            repo_id: repo_id.to_string(),
            author_user_id: owner.id.clone(),
            name: name.to_string(),
            title: Some(title.to_string()),
            author_role: RequestActorRole::Owner,
            audience,
            base_main_oid: base_oid.clone(),
            event_id: format!("event_{id}_started"),
            now_unix,
        },
    )?;
    catalog
        .request_events
        .insert(started.event.id.clone(), started.event);
    record_working_request_upload(
        &mut catalog.requests,
        RecordWorkingRequestUploadInput {
            request_id: id.to_string(),
            actor_user_id: owner.id.clone(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: head_oid.clone(),
            git_snapshot: snapshot,
            now_unix: now_unix + 1,
        },
    )?;
    let submitted = submit_request(
        &mut catalog.requests,
        &mut catalog.request_events,
        &mut catalog.user_credit_accounts,
        &mut catalog.credit_ledger_entries,
        SubmitRequestInput {
            request_id: id.to_string(),
            actor_user_id: owner.id.clone(),
            expected_head_oid: head_oid.clone(),
            stake_credits: 0,
            stake_ledger_entry_id: None,
            event_id: format!("event_{id}_submitted"),
            now_unix: now_unix + 2,
        },
    )?;
    catalog
        .request_change_blocks
        .insert(submitted.change_block.id.clone(), submitted.change_block);
    catalog
        .request_discussions
        .insert(submitted.discussion.id.clone(), submitted.discussion);
    catalog.request_discussion_read_states.insert(
        format!(
            "{}:{}",
            submitted.read_state.discussion_id, submitted.read_state.user_id
        ),
        submitted.read_state,
    );

    match outcome {
        SeedRequestOutcome::Submitted => {}
        SeedRequestOutcome::NeedsResponse => {
            mark_request_needs_response(
                &mut catalog.requests,
                &mut catalog.request_events,
                MarkRequestNeedsResponseInput {
                    request_id: id.to_string(),
                    actor_user_id: owner.id.clone(),
                    event_id: format!("event_{id}_needs_response"),
                    body: "Please add a concrete recovery command before this is merged."
                        .to_string(),
                    now_unix: now_unix + 3,
                },
            )?;
        }
        SeedRequestOutcome::Accepted => {
            merge_request(
                &mut catalog.requests,
                &mut catalog.request_events,
                &mut catalog.user_credit_accounts,
                &mut catalog.credit_ledger_entries,
                MergeRequestInput {
                    request_id: id.to_string(),
                    actor_user_id: owner.id.clone(),
                    expected_main_oid: base_oid.clone(),
                    current_main_oid: base_oid,
                    expected_head_oid: head_oid,
                    event_id: format!("event_{id}_merged"),
                    settlement_event_id: format!("event_{id}_settled"),
                    refund_ledger_entry_id: None,
                    reward_ledger_entry_id: None,
                    body: Some("Merged after review.".to_string()),
                    now_unix: now_unix + 3,
                },
            )?;
        }
        SeedRequestOutcome::Withdrawn => {
            delete_request(
                &mut catalog.requests,
                &mut catalog.request_events,
                &mut catalog.user_credit_accounts,
                &mut catalog.credit_ledger_entries,
                DeleteRequestInput {
                    request_id: id.to_string(),
                    actor_user_id: owner.id.clone(),
                    actor_can_delete: true,
                    event_id: format!("event_{id}_withdrawn"),
                    refund_ledger_entry_id: None,
                    now_unix: now_unix + 3,
                },
            )?;
        }
    }
    Ok(())
}

fn repo(
    owner: &UserAccount,
    name: &str,
    visibility: Visibility,
) -> Result<StoredRepository, ApiError> {
    StoredRepository::new(owner, name, visibility)
        .map_err(|error| ApiError::internal_message(error.to_string()))
}

fn commit(
    repo: &StoredRepository,
    id: &str,
    parent_ids: impl IntoIterator<Item = &'static str>,
    message: &str,
    changes: Vec<FileChange>,
) -> LogicalCommit {
    LogicalCommit {
        id: id.to_string(),
        parent_ids: parent_ids.into_iter().map(ToString::to_string).collect(),
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: message.to_string(),
        changes,
    }
}

fn add_change(
    path: &str,
    new_content: SourceBlob,
    visibility: Visibility,
) -> Result<FileChange, ApiError> {
    Ok(FileChange {
        path: ScopePath::parse(path).map_err(ApiError::internal)?,
        old_content: None,
        new_content: Some(new_content),
        visibility,
    })
}

fn blob(
    object_store: &dyn ObjectStore,
    repo: &StoredRepository,
    content: &str,
) -> Result<SourceBlob, ApiError> {
    Ok(put_source_blob(
        object_store,
        &repo.record.id,
        content.as_bytes(),
    )?)
}

#[derive(Clone, Copy)]
struct SeedGitCommit<'a> {
    files: &'a [(&'a str, &'a str)],
    message: &'a str,
}

fn git_segment_state(
    object_store: &dyn ObjectStore,
    repo: &StoredRepository,
    label: &str,
    commits: &[SeedGitCommit<'_>],
) -> Result<(GitHead, GitSegment), ApiError> {
    with_seed_git_repo(label, |repo_path| {
        apply_seed_commits(repo_path, commits)?;
        store_seed_git_segment(object_store, repo, repo_path)
    })
}

fn update_demo_git_snapshot(
    object_store: &dyn ObjectStore,
    repo: &StoredRepository,
    initial: SeedGitCommit<'_>,
    accepted: SeedGitCommit<'_>,
) -> Result<(GitHead, GitSegment, SeedRequestGallery), ApiError> {
    with_seed_git_repo("update-demo-live", |repo_path| {
        apply_seed_commits(repo_path, &[initial])?;
        let initial_oid = seed_git_head(repo_path)?;
        apply_seed_commits(repo_path, &[accepted])?;
        let main_oid = seed_git_head(repo_path)?;
        let accepted_ref = canonical_request_ref("document-release-flow");
        seed_git(
            Some(repo_path),
            &["update-ref", &accepted_ref, &main_oid],
            "creating seeded request ref",
        )?;

        let submitted_oid = seed_request_branch(
            repo_path,
            "bounded-retry-timing",
            SeedGitCommit {
                files: &[("src/retry.ts", UPDATE_DEMO_RETRY_HELPER)],
                message: "Add bounded retry timing",
            },
            &main_oid,
        )?;
        let needs_response_oid = seed_request_branch(
            repo_path,
            "remote-troubleshooting",
            SeedGitCommit {
                files: &[("docs/troubleshooting.md", UPDATE_DEMO_TROUBLESHOOTING)],
                message: "Add remote troubleshooting",
            },
            &main_oid,
        )?;
        let withdrawn_oid = seed_request_branch(
            repo_path,
            "verbose-cli-output",
            SeedGitCommit {
                files: &[("experiments/cli-output.txt", UPDATE_DEMO_CLI_EXPERIMENT)],
                message: "Try verbose CLI output",
            },
            &main_oid,
        )?;
        let (main_head, main_segment) = store_seed_git_segment(object_store, repo, repo_path)?;
        let submitted_snapshot = store_seed_bundle(
            object_store,
            repo,
            repo_path,
            "req_demo_submitted",
            &[&canonical_request_ref("bounded-retry-timing")],
        )?;
        let needs_response_snapshot = store_seed_bundle(
            object_store,
            repo,
            repo_path,
            "req_demo_needs_response",
            &[&canonical_request_ref("remote-troubleshooting")],
        )?;
        let accepted_snapshot = store_seed_bundle(
            object_store,
            repo,
            repo_path,
            "req_demo_accepted",
            &[&accepted_ref],
        )?;
        let withdrawn_snapshot = store_seed_bundle(
            object_store,
            repo,
            repo_path,
            "req_demo_withdrawn",
            &[&canonical_request_ref("verbose-cli-output")],
        )?;
        let gallery = vec![
            SeedRequest {
                id: "req_demo_submitted",
                name: "bounded-retry-timing",
                title: "Add bounded retry timing",
                base_oid: main_oid.clone(),
                head_oid: submitted_oid,
                snapshot: submitted_snapshot,
                outcome: SeedRequestOutcome::Submitted,
                audience: RequestAudience::Public,
                now_unix: 1_800_000_100,
            },
            SeedRequest {
                id: "req_demo_needs_response",
                name: "remote-troubleshooting",
                title: "Add remote troubleshooting",
                base_oid: main_oid.clone(),
                head_oid: needs_response_oid,
                snapshot: needs_response_snapshot,
                outcome: SeedRequestOutcome::NeedsResponse,
                audience: RequestAudience::Private,
                now_unix: 1_800_000_200,
            },
            SeedRequest {
                id: "req_demo_accepted",
                name: "document-release-flow",
                title: "Document the release flow",
                base_oid: initial_oid,
                head_oid: main_oid.clone(),
                snapshot: accepted_snapshot,
                outcome: SeedRequestOutcome::Accepted,
                audience: RequestAudience::Private,
                now_unix: 1_800_000_300,
            },
            SeedRequest {
                id: "req_demo_withdrawn",
                name: "verbose-cli-output",
                title: "Try verbose CLI output",
                base_oid: main_oid,
                head_oid: withdrawn_oid,
                snapshot: withdrawn_snapshot,
                outcome: SeedRequestOutcome::Withdrawn,
                audience: RequestAudience::Private,
                now_unix: 1_800_000_400,
            },
        ];
        Ok((main_head, main_segment, gallery))
    })
}

fn store_seed_git_segment(
    object_store: &dyn ObjectStore,
    repo: &StoredRepository,
    repo_path: &FsPath,
) -> Result<(GitHead, GitSegment), ApiError> {
    let head_oid = seed_git_head(repo_path)?;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["pack-objects", "--revs", "--stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ApiError::internal)?;
    child
        .stdin
        .take()
        .ok_or_else(|| ApiError::internal_message("seed pack stdin unavailable"))?
        .write_all(format!("{head_oid}\n").as_bytes())
        .map_err(ApiError::internal)?;
    let output = child.wait_with_output().map_err(ApiError::internal)?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "creating seeded Git segment: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let segment_object = put_repo_object(
        object_store,
        &repo.record.id,
        "git-segments",
        &output.stdout,
    )?;
    let manifest = scope_core::git_segments::GitSegmentManifest::new(
        head_oid.clone(),
        None,
        segment_object.clone(),
    );
    let mut manifest_object = put_repo_object(
        object_store,
        &repo.record.id,
        "git-manifests",
        &manifest.encode()?,
    )?;
    manifest_object.git_oid = head_oid.clone();
    Ok((
        GitHead {
            head_oid: head_oid.clone(),
            segment_sequence: 1,
            change_version: 1,
            manifest: manifest_object.clone(),
        },
        GitSegment {
            sequence: 1,
            base_oid: None,
            head_oid,
            object: segment_object,
            manifest: manifest_object,
        },
    ))
}

fn seed_request_branch(
    repo_path: &FsPath,
    request_id: &str,
    commit: SeedGitCommit<'_>,
    main_oid: &str,
) -> Result<String, ApiError> {
    apply_seed_commits(repo_path, &[commit])?;
    let head_oid = seed_git_head(repo_path)?;
    let request_ref = canonical_request_ref(request_id);
    seed_git(
        Some(repo_path),
        &["update-ref", &request_ref, &head_oid],
        "creating seeded request ref",
    )?;
    seed_git(
        Some(repo_path),
        &["reset", "--hard", main_oid],
        "restoring seeded main branch",
    )?;
    Ok(head_oid)
}

fn with_seed_git_repo<T>(
    label: &str,
    build: impl FnOnce(&FsPath) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    let repo_path = temp_seed_git_repo_path(label)?;
    if repo_path.exists() {
        fs::remove_dir_all(&repo_path).map_err(ApiError::internal)?;
    }
    fs::create_dir_all(&repo_path).map_err(ApiError::internal)?;

    let result = (|| {
        seed_git(
            None,
            &["init", repo_path.to_string_lossy().as_ref()],
            "initializing seeded Git repo",
        )?;
        seed_git(
            Some(&repo_path),
            &["checkout", "-B", DEFAULT_GIT_BRANCH],
            "creating seeded default branch",
        )?;
        build(&repo_path)
    })();

    let cleanup = fs::remove_dir_all(&repo_path);
    if let Err(error) = cleanup
        && result.is_ok()
    {
        return Err(ApiError::internal(error));
    }
    result
}

fn apply_seed_commits(repo_path: &FsPath, commits: &[SeedGitCommit<'_>]) -> Result<(), ApiError> {
    for commit in commits {
        for (path, content) in commit.files {
            let path = repo_path.join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(ApiError::internal)?;
            }
            fs::write(path, content).map_err(ApiError::internal)?;
        }
        seed_git(Some(repo_path), &["add", "--all"], "adding seeded files")?;
        seed_git(
            Some(repo_path),
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--no-gpg-sign",
                "--no-verify",
                "--message",
                commit.message,
            ],
            "committing seeded files",
        )?;
    }
    Ok(())
}

fn store_seed_bundle(
    object_store: &dyn ObjectStore,
    repo: &StoredRepository,
    repo_path: &FsPath,
    label: &str,
    refs: &[&str],
) -> Result<SourceBlob, ApiError> {
    let bundle_path = repo_path.join(format!("{label}.bundle"));
    let bundle = bundle_path.to_string_lossy().to_string();
    let mut args = vec!["bundle", "create", bundle.as_str()];
    args.extend_from_slice(refs);
    seed_git(Some(repo_path), &args, "creating seeded Git bundle")?;
    let bytes = fs::read(&bundle_path).map_err(ApiError::internal)?;
    Ok(put_repo_object(
        object_store,
        &repo.record.id,
        "git-bundles",
        &bytes,
    )?)
}

fn seed_git_head(repo_path: &FsPath) -> Result<String, ApiError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| ApiError::service_unavailable(format!("reading seeded head: {error}")))?;
    if !output.status.success() {
        return Err(ApiError::service_unavailable(format!(
            "reading seeded head: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8(output.stdout)
        .map_err(ApiError::internal)?
        .trim()
        .to_string())
}

fn temp_seed_git_repo_path(label: &str) -> Result<std::path::PathBuf, ApiError> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(ApiError::internal)?
        .as_nanos();
    Ok(std::env::temp_dir().join(format!(
        "scope-vcs-dev-seed-{}-{}-{nanos}",
        std::process::id(),
        label
    )))
}

fn seed_git(repo: Option<&FsPath>, args: &[&str], action: &str) -> Result<(), ApiError> {
    let mut command = Command::new("git");
    if let Some(repo) = repo {
        command.arg("-C").arg(repo);
    }
    let output = command
        .args(args)
        .env("GIT_AUTHOR_NAME", "Scope Dev Seed")
        .env("GIT_AUTHOR_EMAIL", "scope-dev@example.invalid")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_NAME", "Scope Dev Seed")
        .env("GIT_COMMITTER_EMAIL", "scope-dev@example.invalid")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .output()
        .map_err(|error| ApiError::service_unavailable(format!("failed {action}: {error}")))?;
    if output.status.success() {
        return Ok(());
    }

    Err(ApiError::service_unavailable(format!(
        "{action}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppState;
    use crate::domain::requests::{RequestDisposition, RequestState};
    use crate::git::import::git_stdout_text;
    use crate::git::storage::restore_git_segments;
    use crate::object_store::{EncryptedObjectStore, MemoryObjectStore, source_blob_bytes};
    use std::sync::Arc;

    #[tokio::test]
    async fn seed_catalog_contains_owned_repos_with_readable_blobs() {
        let store = EncryptedObjectStore::new(Arc::new(MemoryObjectStore::new()), [9; 32]);

        let catalog = super::catalog(
            &store,
            DevSeedUser {
                email: "dev@example.com".to_string(),
                handle: "dev".to_string(),
            },
        )
        .unwrap();

        let repos = catalog.repositories_for_user(DEV_SEED_USER_ID);
        assert_eq!(repos.len(), 2);
        assert!(catalog.repository("dev", "public-demo").is_some());
        assert!(catalog.repository("dev", "update-demo").is_some());
        assert_eq!(
            catalog.users.get(DEV_SEED_CONTRIBUTOR_ID).unwrap().handle,
            "river-contributor"
        );
        assert_eq!(
            catalog.users.get(DEV_SEED_MAINTAINER_ID).unwrap().handle,
            "maya-maintainer"
        );

        let public_demo = catalog.repository("dev", "public-demo").unwrap();
        let readme = public_demo.graph.commits[0].changes[0]
            .new_content
            .as_ref()
            .unwrap();
        let readme_bytes = source_blob_bytes(&store, readme).unwrap();
        assert!(
            std::str::from_utf8(&readme_bytes)
                .unwrap()
                .contains("Public Demo")
        );

        assert_eq!(catalog.requests.len(), 4);
        assert_eq!(
            catalog
                .requests
                .get(DISCUSSION_REQUEST_ID)
                .unwrap()
                .audience,
            RequestAudience::Public
        );
        assert_eq!(
            request_state(&catalog, "req_demo_submitted"),
            RequestState::Submitted
        );
        assert_eq!(
            request_state(&catalog, "req_demo_needs_response"),
            RequestState::NeedsResponse
        );
        let accepted = catalog.requests.get("req_demo_accepted").unwrap();
        assert_eq!(accepted.state, RequestState::Resolved);
        assert_eq!(accepted.disposition, Some(RequestDisposition::Accepted));
        assert_eq!(
            request_state(&catalog, "req_demo_withdrawn"),
            RequestState::Withdrawn
        );
    }

    #[tokio::test]
    async fn seed_catalog_git_segments_restore_raw_repositories() {
        let store = Arc::new(EncryptedObjectStore::new(
            Arc::new(MemoryObjectStore::new()),
            [9; 32],
        ));
        let catalog = super::catalog(
            store.as_ref(),
            DevSeedUser {
                email: "dev@example.com".to_string(),
                handle: "dev".to_string(),
            },
        )
        .unwrap();
        let mut state = AppState::test_state();
        state.object_store = store;
        let target = crate::db::TestDatabaseTarget::required().unwrap();
        state.metadata = crate::db::MetadataStore::connect_fresh_for_tests(&target).unwrap();
        state
            .metadata
            .seed_catalog_for_tests(catalog.clone())
            .unwrap();
        state.data_dir = Arc::new(seed_snapshot_test_data_dir());

        let public_demo = catalog.repository("dev", "public-demo").unwrap();
        assert_snapshot_file(
            &state,
            &public_demo.git_head.as_ref().unwrap().manifest,
            "public-demo-live",
            "README.md",
            PUBLIC_DEMO_README,
        );

        let update_demo = catalog.repository("dev", "update-demo").unwrap();
        assert_snapshot_file(
            &state,
            &update_demo.git_head.as_ref().unwrap().manifest,
            "update-demo-live",
            "README.md",
            UPDATE_DEMO_INITIAL_README,
        );
        for request in catalog.requests.values() {
            let snapshot = request
                .git_snapshot
                .as_ref()
                .expect("seeded requests have Git snapshots");
            let repo_root = state.data_dir.join(format!("request-{}.git", request.name));
            let bundle_path = state
                .data_dir
                .join(format!("request-{}.bundle", request.name));
            fs::create_dir_all(state.data_dir.as_ref()).unwrap();
            fs::write(
                &bundle_path,
                source_blob_bytes(state.object_store.as_ref(), snapshot).unwrap(),
            )
            .unwrap();
            seed_git(
                None,
                &["init", "--bare", repo_root.to_str().unwrap()],
                "initializing seeded request snapshot test repo",
            )
            .unwrap();
            let request_ref = canonical_request_ref(&request.name);
            seed_git(
                Some(&repo_root),
                &[
                    "fetch",
                    bundle_path.to_str().unwrap(),
                    &format!("{request_ref}:{request_ref}"),
                ],
                "restoring seeded named request snapshot",
            )
            .unwrap();
            let actual_head = git_stdout_text(
                &repo_root,
                &["rev-parse", &request_ref],
                "reading seeded named request ref",
            )
            .unwrap();
            assert_eq!(actual_head.trim(), request.head_oid);
            let _ = fs::remove_dir_all(repo_root);
            let _ = fs::remove_file(bundle_path);
        }

        let _ = fs::remove_dir_all(state.data_dir.as_ref());
    }

    fn request_state(catalog: &AppCatalog, request_id: &str) -> RequestState {
        catalog.requests.get(request_id).unwrap().state
    }

    fn assert_snapshot_file(
        state: &AppState,
        snapshot: &SourceBlob,
        label: &str,
        path: &str,
        expected: &str,
    ) {
        let repo_root = state.data_dir.join(format!("{label}.git"));
        restore_git_segments(state, snapshot, &repo_root).unwrap();
        let actual = git_stdout_text(
            &repo_root,
            &["show", &format!("{DEFAULT_GIT_BRANCH}:{path}")],
            "reading seeded snapshot file",
        )
        .unwrap();
        assert_eq!(actual, expected);
        let _ = fs::remove_dir_all(repo_root);
    }

    fn seed_snapshot_test_data_dir() -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "scope-vcs-seed-snapshot-test-{}-{nanos}",
            std::process::id()
        ))
    }
}
