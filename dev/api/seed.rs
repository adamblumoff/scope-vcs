use super::env::DevSeedUser;
use crate::{
    domain::{
        policy::{ScopePath, Visibility, VisibilityRule},
        projection::{AuthorVisibility, FileChange, LogicalCommit, MixedCommitPolicy},
        store::{
            AccountAccess, AppCatalog, LineDiff, PendingImport, PendingImportFile,
            RepoPublicationState, RepoSettings, SourceBlob, StagedFileChange, StagedFileChangeKind,
            StagedRepoUpdate, StoredRepository, UserAccount,
        },
    },
    error::ApiError,
    object_store::{ObjectStore, put_source_blob},
};

const DEV_SEED_USER_ID: &str = "scope_usr_dev_seed";

pub(super) fn catalog(
    object_store: &dyn ObjectStore,
    seed_user: DevSeedUser,
) -> Result<AppCatalog, ApiError> {
    let owner = UserAccount {
        id: DEV_SEED_USER_ID.to_string(),
        handle: seed_user.handle,
        email: seed_user.email,
        email_verified: true,
        access: AccountAccess::Member,
    };
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

fn published_demo(
    object_store: &dyn ObjectStore,
    owner: &UserAccount,
) -> Result<StoredRepository, ApiError> {
    let mut repo = repo(owner, "public-demo", Visibility::Public)?;
    let readme = blob(
        object_store,
        &repo,
        "# Public Demo\n\nThis seeded repository is ready to browse locally.\n",
    )?;
    let app = blob(
        object_store,
        &repo,
        "export function greet(name: string) {\n  return `hello ${name}`\n}\n",
    )?;
    let private_plan = blob(
        object_store,
        &repo,
        "# Internal Plan\n\nPrivate content stays out of public projections.\n",
    )?;
    let private_path = ScopePath::parse("/internal/plan.md").map_err(ApiError::internal)?;
    repo.policy
        .add_rule(VisibilityRule::private(
            private_path.clone(),
            [owner.id.clone()],
        ))
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
    repo.git_snapshot = Some(blob(object_store, &repo, "dev public git snapshot")?);
    Ok(repo)
}

fn pending_publish_demo(
    object_store: &dyn ObjectStore,
    owner: &UserAccount,
) -> Result<StoredRepository, ApiError> {
    let mut repo = repo(owner, "review-demo", Visibility::Private)?;
    let files = [
        (
            "README.md",
            "# Review Demo\n\nThis repo is waiting on a first publish review.\n",
        ),
        (
            "src/lib.rs",
            "pub fn seeded_answer() -> usize {\n    42\n}\n",
        ),
        (
            "docs/private-roadmap.md",
            "# Private Roadmap\n\nReview can decide what becomes public.\n",
        ),
    ];
    repo.pending_import = Some(PendingImport {
        default_branch: "main".to_string(),
        head_oid: "1111111111111111111111111111111111111111".to_string(),
        tree_oid: "2222222222222222222222222222222222222222".to_string(),
        imported_at_unix: 1_800_000_000,
        git_snapshot: blob(object_store, &repo, "dev pending git snapshot")?,
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
    repo.record.publication_state = RepoPublicationState::PendingPublish;
    Ok(repo)
}

fn staged_update_demo(
    object_store: &dyn ObjectStore,
    owner: &UserAccount,
) -> Result<StoredRepository, ApiError> {
    let mut repo = repo(owner, "update-demo", Visibility::Public)?;
    let initial_readme = blob(
        object_store,
        &repo,
        "# Update Demo\n\nThis repository has a clean published baseline.\n",
    )?;
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
    repo.git_snapshot = Some(blob(object_store, &repo, "dev live update git snapshot")?);

    let updated_readme = blob(
        object_store,
        &repo,
        "# Update Demo\n\nThis staged change is waiting for review.\n",
    )?;
    let private_note = blob(
        object_store,
        &repo,
        "# Release Notes\n\nKeep this private until the launch is announced.\n",
    )?;
    repo.staged_update = Some(StagedRepoUpdate {
        id: "staged-dev-update".to_string(),
        branch: "refs/heads/main".to_string(),
        base_live_commit_id: Some("dev-update-1".to_string()),
        author_id: owner.id.clone(),
        message: "Stage local UI review sample".to_string(),
        git_snapshot: blob(object_store, &repo, "dev staged update git snapshot")?,
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
        mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
        changes,
        visibility_changes: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
