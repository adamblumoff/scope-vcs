use super::env::DevSeedUser;
use crate::{
    config::DEFAULT_GIT_BRANCH,
    domain::{
        policy::{ScopePath, Visibility, VisibilityRule},
        projection::{AuthorVisibility, FileChange, LogicalCommit},
        store::{
            AccountAccess, AppCatalog, LineDiff, PendingImport, PendingImportFile,
            RepoPublicationState, RepoSettings, SourceBlob, StagedFileChange, StagedFileChangeKind,
            StagedRepoUpdate, StoredRepository, UserAccount,
        },
    },
    error::ApiError,
    object_store::{ObjectStore, put_repo_object, put_source_blob},
};
use std::{fs, path::Path as FsPath, process::Command};

pub(super) const DEV_SEED_USER_ID: &str = "scope_usr_dev_seed";
const PUBLIC_DEMO_README: &str =
    "# Public Demo\n\nThis seeded repository is ready to browse locally.\n";
const PUBLIC_DEMO_APP: &str =
    "export function greet(name: string) {\n  return `hello ${name}`\n}\n";
const PUBLIC_DEMO_PLAN: &str =
    "# Internal Plan\n\nPrivate content stays out of public projections.\n";
const REVIEW_DEMO_README: &str =
    "# Review Demo\n\nThis repo is waiting on a first publish review.\n";
const REVIEW_DEMO_LIB: &str = "pub fn seeded_answer() -> usize {\n    42\n}\n";
const REVIEW_DEMO_ROADMAP: &str = "# Private Roadmap\n\nReview can decide what becomes public.\n";
const UPDATE_DEMO_INITIAL_README: &str =
    "# Update Demo\n\nThis repository has a clean published baseline.\n";
const UPDATE_DEMO_UPDATED_README: &str =
    "# Update Demo\n\nThis staged change is waiting for review.\n";
const UPDATE_DEMO_RELEASE_NOTES: &str =
    "# Release Notes\n\nKeep this private until the launch is announced.\n";

pub(super) fn catalog(
    object_store: &dyn ObjectStore,
    seed_user: DevSeedUser,
) -> Result<AppCatalog, ApiError> {
    let owner = seed_user_account(seed_user);
    let mut catalog = AppCatalog::default();
    catalog.users.insert(owner.id.clone(), owner.clone());

    for repo in [
        published_demo(object_store, &owner)?,
        pending_publish_demo(object_store, &owner)?,
        staged_update_demo(object_store, &owner)?,
    ] {
        catalog.repositories.insert(repo.record.id.clone(), repo);
    }

    Ok(catalog)
}

pub(super) fn seed_user_account(seed_user: DevSeedUser) -> UserAccount {
    UserAccount {
        id: DEV_SEED_USER_ID.to_string(),
        handle: seed_user.handle,
        email: seed_user.email,
        email_verified: true,
        access: AccountAccess::Member,
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
    repo.record.publication_state = RepoPublicationState::Published;
    repo.settings = RepoSettings {
        include_ignored_files: false,
        review_pushes_before_applying: false,
    };
    repo.git_snapshot = Some(git_snapshot(
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
    )?);
    Ok(repo)
}

fn pending_publish_demo(
    object_store: &dyn ObjectStore,
    owner: &UserAccount,
) -> Result<StoredRepository, ApiError> {
    let mut repo = repo(owner, "review-demo", Visibility::Private)?;
    let files = [
        ("README.md", REVIEW_DEMO_README),
        ("src/lib.rs", REVIEW_DEMO_LIB),
        ("docs/private-roadmap.md", REVIEW_DEMO_ROADMAP),
    ];
    repo.pending_import = Some(PendingImport {
        default_branch: "main".to_string(),
        head_oid: "1111111111111111111111111111111111111111".to_string(),
        tree_oid: "2222222222222222222222222222222222222222".to_string(),
        imported_at_unix: 1_800_000_000,
        git_snapshot: git_snapshot(
            object_store,
            &repo,
            "review-demo-pending",
            &[SeedGitCommit {
                files: &files,
                message: "Seed review demo",
            }],
        )?,
        files: files
            .into_iter()
            .map(|(path, content)| {
                let blob = blob(object_store, &repo, content)?;
                Ok(PendingImportFile {
                    path: path.to_string(),
                    mode: "100644".to_string(),
                    oid: blob.git_oid.clone(),
                    blob,
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()?,
    });
    repo.record.publication_state = RepoPublicationState::Unpublished;
    Ok(repo)
}

fn staged_update_demo(
    object_store: &dyn ObjectStore,
    owner: &UserAccount,
) -> Result<StoredRepository, ApiError> {
    let mut repo = repo(owner, "update-demo", Visibility::Public)?;
    let initial_readme = blob(object_store, &repo, UPDATE_DEMO_INITIAL_README)?;
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
    repo.record.publication_state = RepoPublicationState::Published;
    repo.git_snapshot = Some(git_snapshot(
        object_store,
        &repo,
        "update-demo-live",
        &[SeedGitCommit {
            files: &[("README.md", UPDATE_DEMO_INITIAL_README)],
            message: "Seed update demo",
        }],
    )?);

    let updated_readme = blob(object_store, &repo, UPDATE_DEMO_UPDATED_README)?;
    let private_note = blob(object_store, &repo, UPDATE_DEMO_RELEASE_NOTES)?;
    repo.staged_update = Some(StagedRepoUpdate {
        id: "staged-dev-update".to_string(),
        branch: "refs/heads/main".to_string(),
        base_live_commit_id: Some("dev-update-1".to_string()),
        author_id: owner.id.clone(),
        message: "Stage local UI review sample".to_string(),
        git_snapshot: git_snapshot(
            object_store,
            &repo,
            "update-demo-staged",
            &[
                SeedGitCommit {
                    files: &[("README.md", UPDATE_DEMO_INITIAL_README)],
                    message: "Seed update demo",
                },
                SeedGitCommit {
                    files: &[
                        ("README.md", UPDATE_DEMO_UPDATED_README),
                        ("internal/release-notes.md", UPDATE_DEMO_RELEASE_NOTES),
                    ],
                    message: "Stage local UI review sample",
                },
            ],
        )?,
        changes: vec![
            StagedFileChange {
                path: ScopePath::parse("/README.md").map_err(ApiError::internal)?,
                old_content: Some(initial_readme),
                new_content: Some(updated_readme),
                line_diff: LineDiff {
                    additions: 1,
                    deletions: 1,
                },
                visibility: Visibility::Public,
                kind: StagedFileChangeKind::Modified,
            },
            StagedFileChange {
                path: ScopePath::parse("/internal/release-notes.md").map_err(ApiError::internal)?,
                old_content: None,
                new_content: Some(private_note),
                line_diff: LineDiff {
                    additions: 3,
                    deletions: 0,
                },
                visibility: Visibility::Private,
                kind: StagedFileChangeKind::Added,
            },
        ],
    });
    Ok(repo)
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
    put_source_blob(object_store, &repo.record.id, content.as_bytes())
}

struct SeedGitCommit<'a> {
    files: &'a [(&'a str, &'a str)],
    message: &'a str,
}

fn git_snapshot(
    object_store: &dyn ObjectStore,
    repo: &StoredRepository,
    label: &str,
    commits: &[SeedGitCommit<'_>],
) -> Result<SourceBlob, ApiError> {
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

        for commit in commits {
            for (path, content) in commit.files {
                let path = repo_path.join(path);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(ApiError::internal)?;
                }
                fs::write(path, content).map_err(ApiError::internal)?;
            }
            seed_git(Some(&repo_path), &["add", "--all"], "staging seeded files")?;
            seed_git(
                Some(&repo_path),
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

        let bundle_path = repo_path.join("scope-seed.bundle");
        seed_git(
            Some(&repo_path),
            &[
                "bundle",
                "create",
                bundle_path.to_string_lossy().as_ref(),
                "--all",
            ],
            "creating seeded Git bundle",
        )?;
        let bytes = fs::read(&bundle_path).map_err(ApiError::internal)?;
        put_repo_object(object_store, &repo.record.id, "git-bundles", &bytes)
    })();

    let cleanup = fs::remove_dir_all(&repo_path);
    if let Err(error) = cleanup
        && result.is_ok()
    {
        return Err(ApiError::internal(error));
    }
    result
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
    use crate::git::import::git_stdout_text;
    use crate::git::storage::restore_git_snapshot;
    use crate::object_store::{EncryptedObjectStore, MemoryObjectStore, source_blob_text};
    use std::sync::Arc;

    #[test]
    fn seed_catalog_contains_owned_repos_with_readable_blobs() {
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
        assert_eq!(repos.len(), 3);
        assert!(catalog.repository("dev", "public-demo").is_some());
        assert!(catalog.repository("dev", "review-demo").is_some());
        assert!(catalog.repository("dev", "update-demo").is_some());

        let public_demo = catalog.repository("dev", "public-demo").unwrap();
        let readme = public_demo.graph.commits[0].changes[0]
            .new_content
            .as_ref()
            .unwrap();
        assert!(
            source_blob_text(&store, readme)
                .unwrap()
                .contains("Public Demo")
        );
    }

    #[test]
    fn seed_catalog_git_snapshots_restore_as_bundles() {
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
        state.metadata = crate::db::MetadataStore::memory(catalog.clone());
        state.data_dir = Arc::new(seed_snapshot_test_data_dir());

        let public_demo = catalog.repository("dev", "public-demo").unwrap();
        assert_snapshot_file(
            &state,
            public_demo.git_snapshot.as_ref().unwrap(),
            "public-demo-live",
            "README.md",
            PUBLIC_DEMO_README,
        );

        let review_demo = catalog.repository("dev", "review-demo").unwrap();
        assert_snapshot_file(
            &state,
            &review_demo.pending_import.as_ref().unwrap().git_snapshot,
            "review-demo-pending",
            "src/lib.rs",
            REVIEW_DEMO_LIB,
        );

        let update_demo = catalog.repository("dev", "update-demo").unwrap();
        assert_snapshot_file(
            &state,
            update_demo.git_snapshot.as_ref().unwrap(),
            "update-demo-live",
            "README.md",
            UPDATE_DEMO_INITIAL_README,
        );
        assert_snapshot_file(
            &state,
            &update_demo.staged_update.as_ref().unwrap().git_snapshot,
            "update-demo-staged",
            "internal/release-notes.md",
            UPDATE_DEMO_RELEASE_NOTES,
        );

        let _ = fs::remove_dir_all(state.data_dir.as_ref());
    }

    fn assert_snapshot_file(
        state: &AppState,
        snapshot: &SourceBlob,
        label: &str,
        path: &str,
        expected: &str,
    ) {
        let repo_root = state.data_dir.join(format!("{label}.git"));
        restore_git_snapshot(state, snapshot, &repo_root).unwrap();
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
