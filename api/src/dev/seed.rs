#[path = "seed/git_segments.rs"]
mod git_segments;
#[path = "seed/request_discussions.rs"]
mod request_discussions;
#[path = "seed/request_revisions.rs"]
mod request_revisions;
#[cfg(any(test, feature = "local-dev"))]
#[path = "seed/runs.rs"]
mod runs;
#[cfg(test)]
#[path = "seed/tests.rs"]
mod tests;
#[path = "seed/workflow_files.rs"]
mod workflow_files;
use git_segments::store_seed_git_pack;
#[cfg(test)]
pub(super) use git_segments::test_seed_git_segment_store;
#[cfg(any(feature = "local-dev", feature = "smoke-seed"))]
pub(crate) use request_discussions::seed_request_discussion_gallery;
use request_revisions::SeedRequestRevision;
#[cfg(feature = "local-dev")]
pub(crate) use runs::seed_run_gallery;
use workflow_files::{PUBLIC_DEMO_CHECKS_WORKFLOW, PUBLIC_DEMO_LINT_WORKFLOW};

use crate::{config::DEFAULT_GIT_BRANCH, error::ApiError};
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    policy::{ScopePath, Visibility, VisibilityRule},
    projection::LogicalCommitOrigin,
    projection::{FileChange, LogicalCommit},
    repository::git::{GitHead, GitPackSpan, GitSegmentUpload},
    repository::{RepoLifecycleState, Repository},
    requests::{
        EditRequestIdentityInput, RecordRequestRevisionInput, RecordWorkingRequestUploadInput,
        RequestActorRole, RequestAudience, StartRequestInput, canonical_request_ref,
        edit_request_identity, record_request_revision, record_working_request_upload,
        start_request,
    },
};
use scope_object_store::{ContentObjectKind, ObjectStore, put_content_object, put_source_blob};
use std::{
    fs,
    path::Path as FsPath,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static SEED_TEMP_REPO_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevSeedUser {
    pub(crate) email: String,
    pub(crate) handle: String,
}

pub(crate) const DEV_SEED_USER_ID: &str = "scope_usr_dev_seed";
const PUBLIC_DEMO_README_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Scope public demo</title>
  <style>
    :root {
      color-scheme: light dark;
      font-family: Inter, ui-sans-serif, system-ui, sans-serif;
      color: #17201d;
      background: #f4f1e9;
    }

    * { box-sizing: border-box; }

    body {
      margin: 0;
      min-height: 100vh;
      background:
        radial-gradient(circle at top right, rgba(76, 115, 93, .2), transparent 42rem),
        #f4f1e9;
    }

    main {
      width: min(100% - 2rem, 62rem);
      margin-inline: auto;
      padding: clamp(4rem, 10vw, 8rem) 0;
    }

    .eyebrow {
      margin: 0 0 1rem;
      color: #4c735d;
      font-size: .75rem;
      font-weight: 700;
      letter-spacing: .16em;
      text-transform: uppercase;
    }

    h1 {
      max-width: 12ch;
      margin: 0;
      font-family: Georgia, serif;
      font-size: clamp(3.25rem, 10vw, 7.5rem);
      font-weight: 500;
      letter-spacing: -.055em;
      line-height: .9;
    }

    .intro {
      max-width: 42rem;
      margin: 2rem 0 0;
      color: #4d5a55;
      font-size: clamp(1rem, 2vw, 1.25rem);
      line-height: 1.7;
    }

    .details {
      display: grid;
      grid-template-columns: repeat(3, 1fr);
      gap: 1.5rem;
      margin-top: clamp(4rem, 9vw, 7rem);
      padding-top: 1.5rem;
      border-top: 1px solid rgba(23, 32, 29, .22);
    }

    .details p { margin: .45rem 0 0; line-height: 1.5; }
    .details strong { font-size: .8rem; letter-spacing: .04em; text-transform: uppercase; }
    code { font: .9em ui-monospace, SFMono-Regular, Consolas, monospace; }

    @media (max-width: 40rem) {
      .details { grid-template-columns: 1fr; }
    }

    @media (prefers-color-scheme: dark) {
      :root { color: #e7ece8; background: #111614; }
      body {
        background:
          radial-gradient(circle at top right, rgba(116, 170, 139, .16), transparent 36rem),
          #111614;
      }
      .eyebrow { color: #8fc5a6; }
      .intro { color: #aebbb4; }
      .details { border-color: rgba(231, 236, 232, .2); }
    }
  </style>
</head>
<body>
  <main>
    <p class="eyebrow">Scope public demo</p>
    <h1>Public by design.</h1>
    <p class="intro">
      A repository can publish a clear, expressive front door without exposing the work that
      belongs behind it. This page is the committed <code>README.html</code>, rendered as-is.
    </p>
    <section class="details" aria-label="Repository details">
      <div>
        <strong>Visible</strong>
        <p>The homepage and a tiny TypeScript example.</p>
      </div>
      <div>
        <strong>Private</strong>
        <p>Internal planning stays out of the public projection.</p>
      </div>
      <div>
        <strong>Portable</strong>
        <p>No build, scripts, SDK, or remote assets required.</p>
      </div>
    </section>
  </main>
</body>
</html>
"#;
const PUBLIC_DEMO_APP: &str =
    "export function greet(name: string) {\n  return `hello ${name}`\n}\n";
const PUBLIC_DEMO_PLAN: &str =
    "# Internal Plan\n\nPrivate content stays out of public projections.\n";
const UPDATE_DEMO_INITIAL_README: &str = "# Update Demo\n\nThis repository has a clean published baseline.\n\n[Read the release guide](docs/release.md).\n";
const UPDATE_DEMO_RULES: &str = "";
const UPDATE_DEMO_RELEASE_GUIDE: &str =
    "# Release flow\n\nDocument the release before publishing the next version.\n";
const UPDATE_DEMO_INTERNAL_NOTES: &str =
    "# Internal notes\n\nOnly repository maintainers can read this file.\n";
const UPDATE_DEMO_RETRY_HELPER: &str =
    "export function retryDelay(attempt: number) {\n  return Math.min(attempt * 250, 2000)\n}\n";
const UPDATE_DEMO_TROUBLESHOOTING: &str =
    "# Troubleshooting\n\nExplain how to recover when the remote is unavailable.\n";
const UPDATE_DEMO_CLI_EXPERIMENT: &str =
    "experimental output: checking repository state before push\n";
const UPDATE_DEMO_QUEUE_DRAFT: &str =
    "# Request queue copy\n\nTighten the language before asking for review.\n";
const UPDATE_DEMO_CACHE_NOTE: &str =
    "# Cache note\n\nRecord the tradeoff without changing repository behavior.\n";

pub(crate) fn catalog(
    object_store: &dyn ObjectStore,
    git_segment_store: &scope_git_storage::GitSegmentStore,
    seed_user: DevSeedUser,
) -> Result<scope_postgres::db::CatalogFixture, ApiError> {
    let owner = seed_user_account(seed_user);
    let [contributor, maintainer] = request_discussions::collaborators();
    let mut catalog = scope_postgres::db::CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner.clone());
    catalog
        .users
        .insert(contributor.id.clone(), contributor.clone());
    catalog
        .users
        .insert(maintainer.id.clone(), maintainer.clone());

    let (mut update_demo, request_gallery, update_segment) =
        update_demo(object_store, git_segment_store, &owner)?;
    request_discussions::add_maintainer(&mut update_demo);
    let (published_demo, published_segment) =
        published_demo(object_store, git_segment_store, &owner)?;
    catalog
        .git_segment_uploads
        .extend([published_segment, update_segment]);
    for repo in [published_demo, update_demo] {
        catalog.repositories.insert(repo.record.id.clone(), repo);
    }
    seed_request_gallery(&mut catalog, &owner, request_gallery)?;

    Ok(catalog)
}

pub(crate) fn seed_user_account(seed_user: DevSeedUser) -> UserAccount {
    UserAccount {
        id: DEV_SEED_USER_ID.to_string(),
        handle: seed_user.handle,
        email: seed_user.email,
        email_verified: true,
    }
}

#[cfg(any(test, feature = "local-dev"))]
pub(crate) fn actor_account(seed_user: DevSeedUser, handle: &str) -> Option<UserAccount> {
    let owner = seed_user_account(seed_user);
    if owner.handle == handle {
        return Some(owner);
    }
    request_discussions::collaborators()
        .into_iter()
        .find(|actor| actor.handle == handle)
}

fn published_demo(
    object_store: &dyn ObjectStore,
    git_segment_store: &scope_git_storage::GitSegmentStore,
    owner: &UserAccount,
) -> Result<(Repository, GitSegmentUpload), ApiError> {
    let mut repo = repo(owner, "public-demo", Visibility::Public)?;
    let readme = blob(object_store, PUBLIC_DEMO_README_HTML)?;
    let app = blob(object_store, PUBLIC_DEMO_APP)?;
    let private_plan = blob(object_store, PUBLIC_DEMO_PLAN)?;
    let checks_workflow = blob(object_store, PUBLIC_DEMO_CHECKS_WORKFLOW)?;
    let lint_workflow = blob(object_store, PUBLIC_DEMO_LINT_WORKFLOW)?;
    let private_path = ScopePath::parse("/internal/plan.md").map_err(ApiError::internal)?;
    repo.policy
        .add_rule(VisibilityRule::private(private_path.clone()))
        .map_err(ApiError::internal)?;
    repo.graph.commits.push(commit(
        &repo,
        "dev-public-1",
        "Seed public demo",
        vec![
            add_change("/README.html", readme, Visibility::Public)?,
            add_change("/src/app.ts", app, Visibility::Public)?,
            add_change(
                "/.scope/runs/checks.yml",
                checks_workflow,
                Visibility::Public,
            )?,
            add_change("/.scope/runs/lint.yml", lint_workflow, Visibility::Public)?,
            add_change(private_path.as_str(), private_plan, Visibility::Private)?,
        ],
    ));
    populate_seed_live_files(&mut repo);
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    let (head, pack_span, segment_upload) = git_pack_state(
        object_store,
        git_segment_store,
        &repo.record.id,
        "public-demo-live",
        &[SeedGitCommit {
            files: &[
                ("README.html", PUBLIC_DEMO_README_HTML),
                ("src/app.ts", PUBLIC_DEMO_APP),
                (".scope/runs/checks.yml", PUBLIC_DEMO_CHECKS_WORKFLOW),
                (".scope/runs/lint.yml", PUBLIC_DEMO_LINT_WORKFLOW),
                ("internal/plan.md", PUBLIC_DEMO_PLAN),
            ],
            message: "Seed public demo",
        }],
    )?;
    repo.git_head = Some(head);
    repo.git_pack_spans.push(pack_span);
    Ok((repo, segment_upload))
}

fn update_demo(
    object_store: &dyn ObjectStore,
    git_segment_store: &scope_git_storage::GitSegmentStore,
    owner: &UserAccount,
) -> Result<(Repository, SeedRequestGallery, GitSegmentUpload), ApiError> {
    let mut repo = repo(owner, "update-demo", Visibility::Public)?;
    let initial_readme = blob(object_store, UPDATE_DEMO_INITIAL_README)?;
    let rules = blob(object_store, UPDATE_DEMO_RULES)?;
    let internal_notes = blob(object_store, UPDATE_DEMO_INTERNAL_NOTES)?;
    let internal_path = ScopePath::parse("/internal/notes.md").map_err(ApiError::internal)?;
    repo.policy
        .add_rule(VisibilityRule::private(internal_path.clone()))
        .map_err(ApiError::internal)?;
    let release_guide = blob(object_store, UPDATE_DEMO_RELEASE_GUIDE)?;
    repo.graph.commits.push(commit(
        &repo,
        "dev-update-1",
        "Seed update demo",
        vec![
            add_change("/README.md", initial_readme.clone(), Visibility::Public)?,
            add_change("/.scope/RULES.md", rules, Visibility::Public)?,
            add_change(internal_path.as_str(), internal_notes, Visibility::Private)?,
        ],
    ));
    repo.graph.commits.push(commit(
        &repo,
        "dev-update-2",
        "Document release flow",
        vec![add_change(
            "/docs/release.md",
            release_guide,
            Visibility::Public,
        )?],
    ));
    populate_seed_live_files(&mut repo);
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    let initial = SeedGitCommit {
        files: &[
            ("README.md", UPDATE_DEMO_INITIAL_README),
            (".scope/RULES.md", UPDATE_DEMO_RULES),
            ("internal/notes.md", UPDATE_DEMO_INTERNAL_NOTES),
        ],
        message: "Seed update demo",
    };
    let accepted = SeedGitCommit {
        files: &[("docs/release.md", UPDATE_DEMO_RELEASE_GUIDE)],
        message: "Document release flow",
    };
    let (head, pack_span, gallery, segment_upload) = update_demo_git_snapshot(
        object_store,
        git_segment_store,
        &repo.record.id,
        initial,
        accepted,
    )?;
    repo.git_head = Some(head);
    repo.git_pack_spans.push(pack_span);
    Ok((repo, gallery, segment_upload))
}

fn populate_seed_live_files(repo: &mut Repository) {
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
    description_markdown: Option<&'static str>,
    revisions: Vec<SeedRequestRevision>,
    outcome: SeedRequestOutcome,
    audience: RequestAudience,
    now_unix: u64,
}

enum SeedRequestOutcome {
    Draft,
    Open,
    Merged,
    Closed,
}

fn seed_request_gallery(
    catalog: &mut scope_postgres::db::CatalogFixture,
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
    catalog: &mut scope_postgres::db::CatalogFixture,
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
        description_markdown,
        revisions,
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
    if let Some(description_markdown) = description_markdown {
        let mutation = edit_request_identity(
            &mut catalog.requests,
            &mut catalog.request_events,
            EditRequestIdentityInput {
                request_id: id.to_string(),
                actor_user_id: owner.id.clone(),
                actor_can_edit_identity: true,
                event_id: format!("event_{id}_identity_edited"),
                title: None,
                description_markdown: Some(description_markdown.to_string()),
                now_unix: now_unix + 2,
            },
        )?;
        catalog
            .request_events
            .insert(mutation.event.id.clone(), mutation.event);
    }
    let mut current_head_oid = head_oid.clone();
    let mut lifecycle_at_unix = now_unix + 2;
    for (index, revision) in revisions.into_iter().enumerate() {
        let mutation = record_request_revision(
            &mut catalog.requests,
            &mut catalog.request_events,
            RecordRequestRevisionInput {
                request_id: id.to_string(),
                actor_user_id: owner.id.clone(),
                actor_can_edit: true,
                expected_old_head_oid: Some(current_head_oid),
                new_head_oid: revision.head_oid.clone(),
                git_snapshot: revision.snapshot,
                event_id: format!("event_{id}_revision_{}", index + 1),
                body: Some(revision.note.to_string()),
                now_unix: now_unix + 3 + index as u64,
            },
        )?;
        current_head_oid = revision.head_oid;
        lifecycle_at_unix = now_unix + 4 + index as u64;
        catalog
            .request_revisions
            .insert(mutation.revision.id.clone(), mutation.revision);
    }

    let request = catalog.requests.get_mut(id).expect("seed request exists");
    if !matches!(outcome, SeedRequestOutcome::Draft) {
        request.submitted_at_unix = Some(lifecycle_at_unix);
    }
    match outcome {
        SeedRequestOutcome::Draft | SeedRequestOutcome::Open => {}
        SeedRequestOutcome::Merged => {
            request.merged_at_unix = Some(lifecycle_at_unix + 1);
            request.merged_by_user_id = Some(owner.id.clone());
            request.merged_head_oid = Some(current_head_oid.clone());
            request.merged_main_oid = Some(current_head_oid);
        }
        SeedRequestOutcome::Closed => {
            request.closed_at_unix = Some(lifecycle_at_unix + 1);
            request.closed_by_user_id = Some(owner.id.clone());
        }
    }
    request.updated_at_unix = match outcome {
        SeedRequestOutcome::Draft | SeedRequestOutcome::Open => lifecycle_at_unix,
        SeedRequestOutcome::Merged | SeedRequestOutcome::Closed => lifecycle_at_unix + 1,
    };
    request.validate_facts()?;
    Ok(())
}

fn repo(owner: &UserAccount, name: &str, visibility: Visibility) -> Result<Repository, ApiError> {
    Repository::new(
        owner,
        name,
        visibility,
        format!(
            "repoi_seed_{}",
            scope_domain::repository::repo_id(&owner.handle, name)
        ),
    )
    .map_err(|error| ApiError::internal_message(error.to_string()))
}

fn commit(repo: &Repository, id: &str, message: &str, changes: Vec<FileChange>) -> LogicalCommit {
    LogicalCommit {
        id: id.to_string(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: id.to_string(),
        },
        author_id: repo.record.owner_user_id.clone(),
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

fn blob(object_store: &dyn ObjectStore, content: &str) -> Result<SourceBlob, ApiError> {
    Ok(put_source_blob(object_store, content.as_bytes())?)
}

#[derive(Clone, Copy)]
struct SeedGitCommit<'a> {
    files: &'a [(&'a str, &'a str)],
    message: &'a str,
}

fn git_pack_state(
    object_store: &dyn ObjectStore,
    git_segment_store: &scope_git_storage::GitSegmentStore,
    repository_id: &str,
    label: &str,
    commits: &[SeedGitCommit<'_>],
) -> Result<(GitHead, GitPackSpan, GitSegmentUpload), ApiError> {
    with_seed_git_repo(label, |repo_path| {
        apply_seed_commits(repo_path, commits)?;
        store_seed_git_pack(object_store, git_segment_store, repository_id, repo_path)
    })
}

fn update_demo_git_snapshot(
    object_store: &dyn ObjectStore,
    git_segment_store: &scope_git_storage::GitSegmentStore,
    repository_id: &str,
    initial: SeedGitCommit<'_>,
    accepted: SeedGitCommit<'_>,
) -> Result<(GitHead, GitPackSpan, SeedRequestGallery, GitSegmentUpload), ApiError> {
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

        let ready_oid = seed_request_branch(
            repo_path,
            "bounded-retry-timing",
            SeedGitCommit {
                files: &[("src/retry.ts", UPDATE_DEMO_RETRY_HELPER)],
                message: "Add bounded retry timing",
            },
            &main_oid,
        )?;
        let ready_snapshot = store_seed_bundle(
            object_store,
            repo_path,
            "req_demo_ready_0",
            &[&canonical_request_ref("bounded-retry-timing")],
            &ready_oid,
        )?;
        let ready_revisions = request_revisions::seed_bounded_retry_revisions(
            object_store,
            repo_path,
            &ready_oid,
            &main_oid,
        )?;
        let held_oid = seed_request_branch(
            repo_path,
            "remote-troubleshooting",
            SeedGitCommit {
                files: &[("docs/troubleshooting.md", UPDATE_DEMO_TROUBLESHOOTING)],
                message: "Add remote troubleshooting",
            },
            &main_oid,
        )?;
        let rejected_oid = seed_request_branch(
            repo_path,
            "verbose-cli-output",
            SeedGitCommit {
                files: &[("experiments/cli-output.txt", UPDATE_DEMO_CLI_EXPERIMENT)],
                message: "Try verbose CLI output",
            },
            &main_oid,
        )?;
        let working_oid = seed_request_branch(
            repo_path,
            "request-queue-copy",
            SeedGitCommit {
                files: &[("docs/request-queue.md", UPDATE_DEMO_QUEUE_DRAFT)],
                message: "Draft request queue copy",
            },
            &main_oid,
        )?;
        let neutral_oid = seed_request_branch(
            repo_path,
            "cache-observability-note",
            SeedGitCommit {
                files: &[("docs/cache-note.md", UPDATE_DEMO_CACHE_NOTE)],
                message: "Document cache tradeoff",
            },
            &main_oid,
        )?;
        let (main_head, main_pack_span, segment_upload) =
            store_seed_git_pack(object_store, git_segment_store, repository_id, repo_path)?;
        let working_snapshot = store_seed_bundle(
            object_store,
            repo_path,
            "req_demo_working",
            &[&canonical_request_ref("request-queue-copy")],
            &working_oid,
        )?;
        let held_snapshot = store_seed_bundle(
            object_store,
            repo_path,
            "req_demo_held",
            &[&canonical_request_ref("remote-troubleshooting")],
            &held_oid,
        )?;
        let accepted_snapshot = store_seed_bundle(
            object_store,
            repo_path,
            "req_demo_accepted",
            &[&accepted_ref],
            &main_oid,
        )?;
        let rejected_snapshot = store_seed_bundle(
            object_store,
            repo_path,
            "req_demo_rejected",
            &[&canonical_request_ref("verbose-cli-output")],
            &rejected_oid,
        )?;
        let neutral_snapshot = store_seed_bundle(
            object_store,
            repo_path,
            "req_demo_neutral",
            &[&canonical_request_ref("cache-observability-note")],
            &neutral_oid,
        )?;
        let gallery = vec![
            SeedRequest {
                id: "req_demo_working",
                name: "request-queue-copy",
                title: "Tighten request queue copy",
                base_oid: main_oid.clone(),
                head_oid: working_oid,
                snapshot: working_snapshot,
                description_markdown: Some("A private working draft for the request author."),
                revisions: Vec::new(),
                outcome: SeedRequestOutcome::Draft,
                audience: RequestAudience::Public,
                now_unix: 1_800_000_050,
            },
            SeedRequest {
                id: "req_demo_ready",
                name: "bounded-retry-timing",
                title: "Add bounded retry timing",
                base_oid: main_oid.clone(),
                head_oid: ready_oid,
                snapshot: ready_snapshot,
                description_markdown: Some(request_discussions::READY_REQUEST_DESCRIPTION),
                revisions: ready_revisions,
                outcome: SeedRequestOutcome::Open,
                audience: RequestAudience::Public,
                now_unix: 1_800_000_100,
            },
            SeedRequest {
                id: "req_demo_held",
                name: "remote-troubleshooting",
                title: "Add remote troubleshooting",
                base_oid: main_oid.clone(),
                head_oid: held_oid,
                snapshot: held_snapshot,
                description_markdown: None,
                revisions: Vec::new(),
                outcome: SeedRequestOutcome::Open,
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
                description_markdown: None,
                revisions: Vec::new(),
                outcome: SeedRequestOutcome::Merged,
                audience: RequestAudience::Private,
                now_unix: 1_800_000_300,
            },
            SeedRequest {
                id: "req_demo_rejected",
                name: "verbose-cli-output",
                title: "Try verbose CLI output",
                base_oid: main_oid.clone(),
                head_oid: rejected_oid,
                snapshot: rejected_snapshot,
                description_markdown: None,
                revisions: Vec::new(),
                outcome: SeedRequestOutcome::Closed,
                audience: RequestAudience::Private,
                now_unix: 1_800_000_400,
            },
            SeedRequest {
                id: "req_demo_neutral",
                name: "cache-observability-note",
                title: "Document the cache tradeoff",
                base_oid: main_oid,
                head_oid: neutral_oid,
                snapshot: neutral_snapshot,
                description_markdown: Some("A public request kept as closed history."),
                revisions: Vec::new(),
                outcome: SeedRequestOutcome::Closed,
                audience: RequestAudience::Public,
                now_unix: 1_800_000_500,
            },
        ];
        Ok((main_head, main_pack_span, gallery, segment_upload))
    })
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
    repo_path: &FsPath,
    label: &str,
    refs: &[&str],
    head_oid: &str,
) -> Result<SourceBlob, ApiError> {
    let bundle_path = repo_path.join(format!("{label}.bundle"));
    let bundle = bundle_path.to_string_lossy().to_string();
    let mut args = vec!["bundle", "create", bundle.as_str()];
    args.extend_from_slice(refs);
    seed_git(Some(repo_path), &args, "creating seeded Git bundle")?;
    let bytes = fs::read(&bundle_path).map_err(ApiError::internal)?;
    fs::remove_file(&bundle_path).map_err(ApiError::internal)?;
    let mut snapshot = put_content_object(object_store, ContentObjectKind::GitBundle, bytes)?;
    snapshot.git_oid = head_oid.to_string();
    Ok(snapshot)
}

fn seed_git_head(repo_path: &FsPath) -> Result<String, ApiError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| {
            ApiError::infrastructure_unavailable(format!("reading seeded head: {error}"))
        })?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
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
    let sequence = SEED_TEMP_REPO_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(std::env::temp_dir().join(format!(
        "scope-vcs-dev-seed-{}-{}-{nanos}-{sequence}",
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
        .map_err(|error| {
            ApiError::infrastructure_unavailable(format!("failed {action}: {error}"))
        })?;
    if output.status.success() {
        return Ok(());
    }

    Err(ApiError::infrastructure_unavailable(format!(
        "{action}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}
