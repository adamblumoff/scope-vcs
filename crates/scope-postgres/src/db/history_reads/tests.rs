use super::*;
use crate::db::{CatalogFixture, MetadataStore, TestDatabaseTarget};
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    content_ref::ContentRef,
    history::history_view,
    policy::{ScopePath, Visibility},
    projection::{FileChange, LogicalCommit, LogicalCommitOrigin},
    repository::RepoLifecycleState,
};
use std::time::{Duration, Instant};

fn fixture(commits: usize) -> (MetadataStore, Repository) {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let owner = UserAccount {
        id: "history_owner".into(),
        handle: "owner".into(),
        email: "history@example.com".into(),
        email_verified: true,
    };
    let mut repo = Repository::new(&owner, "history", Visibility::Public, "repoi_history").unwrap();
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    for index in 0..commits {
        let oid = format!("{:040x}", index + 1);
        repo.graph.commits.push(LogicalCommit {
            id: format!("logical_{index}"),
            origin: LogicalCommitOrigin::CanonicalPush {
                source_head_oid: oid.clone(),
            },
            author_id: owner.id.clone(),
            message: format!("Change {index}"),
            changes: vec![FileChange {
                path: ScopePath::parse(format!("/file-{}.txt", index % 32)).unwrap(),
                old_content: None,
                new_content: Some(SourceBlob {
                    content_ref: ContentRef::git_bundle_sha256(format!("bundle-{index}")),
                    sha256: format!("hash-{index}"),
                    git_oid: oid,
                    git_file_mode: "100644".into(),
                    size_bytes: 100,
                }),
                visibility: if index % 2 == 0 {
                    Visibility::Public
                } else {
                    Visibility::Private
                },
            }],
        });
    }
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog
        .repositories
        .insert(repo.record.id.clone(), repo.clone());
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    (store, repo)
}

#[tokio::test]
async fn history_pages_match_domain_projection_and_do_not_read_history_when_warm() {
    let (store, repo) = fixture(1000);
    // Seed helpers may build read models. Force an actual cold miss at this frontier.
    store
        .db
        .execute_unprepared("DELETE FROM scope_repository_history_views")
        .await
        .unwrap();
    let baseline = Instant::now();
    let hydrated = store
        .repositories()
        .repository("owner", "history")
        .await
        .unwrap()
        .unwrap();
    let expected_private = history_view(
        &hydrated.graph,
        &hydrated.visibility_change_sets,
        ProjectionViewKey::Private,
    );
    let baseline_elapsed = baseline.elapsed();
    let cold = Instant::now();
    let first = store
        .repositories()
        .repository_history_page(RepositoryHistoryQuery {
            incarnation: &repo.incarnation(),
            version: repo.record.change_version,
            audience: ProjectionViewKey::Private,
            before_source_id: None,
            entry_source_id: None,
            limit: 50,
        })
        .await
        .unwrap();
    let cold_elapsed = cold.elapsed();
    assert!(first.has_more);
    assert_eq!(first.view.entries, expected_private.entries[..50]);
    assert_eq!(first.view.generation, expected_private.generation);

    let held = store.db.begin().await.unwrap();
    held.execute_unprepared("LOCK TABLE scope_logical_commits, scope_file_changes, scope_live_files IN ACCESS EXCLUSIVE MODE").await.unwrap();
    let warm = Instant::now();
    let next = tokio::time::timeout(
        Duration::from_secs(2),
        store
            .repositories()
            .repository_history_page(RepositoryHistoryQuery {
                incarnation: &repo.incarnation(),
                version: repo.record.change_version,
                audience: ProjectionViewKey::Private,
                before_source_id: Some(&first.view.entries[49].source_id),
                entry_source_id: None,
                limit: 50,
            }),
    )
    .await
    .expect("warm pages must not hydrate source history")
    .unwrap();
    let warm_elapsed = warm.elapsed();
    assert_eq!(next.view.entries, expected_private.entries[50..100]);
    let public_access = tokio::time::timeout(
        Duration::from_secs(2),
        store
            .repositories()
            .repository_read_access("owner", "history", None),
    )
    .await
    .expect("public access must reuse current projection facts")
    .unwrap()
    .unwrap();
    assert!(public_access.can_read_root());
    let expected_public = history_view(
        &repo.graph,
        &repo.visibility_change_sets,
        ProjectionViewKey::Public,
    );
    let public = store
        .repositories()
        .repository_history_page(RepositoryHistoryQuery {
            incarnation: &repo.incarnation(),
            version: repo.record.change_version,
            audience: ProjectionViewKey::Public,
            before_source_id: None,
            entry_source_id: None,
            limit: 50,
        })
        .await
        .unwrap();
    assert_eq!(public.view.entries, expected_public.entries[..50]);
    assert_eq!(public.view.generation, expected_public.generation);
    let detail = store
        .repositories()
        .repository_history_page(RepositoryHistoryQuery {
            incarnation: &repo.incarnation(),
            version: repo.record.change_version,
            audience: ProjectionViewKey::Public,
            before_source_id: None,
            entry_source_id: Some(&public.view.entries[10].source_id),
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(detail.view.entries, vec![public.view.entries[10].clone()]);
    held.rollback().await.unwrap();
    eprintln!(
        "1000 commits/32 paths: baseline hydrate+private projection={baseline_elapsed:?}, cold build both audiences={cold_elapsed:?}, warm 50-entry page={warm_elapsed:?}"
    );
}

#[tokio::test]
async fn history_reads_reject_changed_frontiers_and_deleted_boundaries() {
    let (store, repo) = fixture(4);
    store
        .repositories()
        .ensure_history_view(&repo.incarnation())
        .await
        .unwrap();
    store
        .db
        .execute_unprepared("UPDATE scope_repository_history_views SET identity_version=-1")
        .await
        .unwrap();
    store
        .repositories()
        .ensure_history_view(&repo.incarnation())
        .await
        .unwrap();
    assert!(
        history_view_metadata(
            store.db.as_ref(),
            &repo.record.id,
            repo.record.change_version,
            ProjectionViewKey::Private
        )
        .await
        .unwrap()
        .is_some()
    );
    let missing = store
        .repositories()
        .repository_history_page(RepositoryHistoryQuery {
            incarnation: &repo.incarnation(),
            version: repo.record.change_version,
            audience: ProjectionViewKey::Private,
            before_source_id: Some("missing"),
            entry_source_id: None,
            limit: 50,
        })
        .await;
    assert!(missing.is_err());
    store.db.execute_unprepared("UPDATE scope_repositories SET change_version=change_version+1 WHERE id='owner/history'").await.unwrap();
    assert!(
        store
            .repositories()
            .repository_history_page(RepositoryHistoryQuery {
                incarnation: &repo.incarnation(),
                version: repo.record.change_version,
                audience: ProjectionViewKey::Private,
                before_source_id: None,
                entry_source_id: None,
                limit: 50
            })
            .await
            .is_err()
    );
    let current = store
        .repositories()
        .repository_access("owner", "history", Some("history_owner"))
        .await
        .unwrap()
        .unwrap();
    store.db.execute_unprepared("UPDATE scope_repositories SET incarnation_id='repoi_recreated' WHERE id='owner/history'").await.unwrap();
    assert!(
        store
            .repositories()
            .repository_main_oid(&current)
            .await
            .is_err()
    );
    assert!(
        store
            .repositories()
            .repository_policy(&current)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn narrow_access_preserves_membership_lifecycle_and_public_root_capabilities() {
    use scope_domain::{
        policy::{Policy, Principal, VisibilityRule},
        repository::collaboration::{RepositoryMember, RepositoryMemberPermissions},
    };
    let (store, mut repo) = fixture(4);
    repo.policy = Policy::new(Visibility::Private);
    repo.policy
        .add_rule(VisibilityRule::public(
            ScopePath::parse("/file-0.txt").unwrap(),
        ))
        .unwrap();
    repo.members.push(RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: "member".into(),
        permissions: RepositoryMemberPermissions {
            can_push: true,
            can_change_file_visibility: false,
            can_apply_changes: false,
        },
        created_at_unix: 1,
        updated_at_unix: 1,
    });
    store
        .repositories()
        .replace_repository_for_tests(repo.clone())
        .await
        .unwrap();
    for user in [
        None,
        Some("history_owner"),
        Some("member"),
        Some("outsider"),
    ] {
        let narrow = store
            .repositories()
            .repository_read_access("owner", "history", user)
            .await
            .unwrap()
            .unwrap();
        let expected = user
            .map(|user| repo.access_for_user_id(user))
            .unwrap_or_else(scope_domain::repository::access::RepositoryAccess::public);
        assert_eq!(narrow.access, expected);
        assert_eq!(
            narrow.root_visibility,
            repo.policy.effective_visibility(&ScopePath::root())
        );
        if user.is_none() {
            assert!(!narrow.can_read_root());
        }
    }
    assert!(
        scope_domain::projection_views::has_visible_projected_non_control_files(
            &repo,
            &Principal::public()
        )
    );
    store.db.execute_unprepared("UPDATE scope_repositories SET publication_state='AwaitingFirstPush', change_version=change_version+1 WHERE id='owner/history'").await.unwrap();
    assert!(
        store
            .repositories()
            .repository_read_access("owner", "history", None)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .repositories()
            .repository_read_access("owner", "history", Some("member"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .repositories()
            .repository_access("owner", "history", Some("member"))
            .await
            .unwrap()
            .unwrap()
            .ensure_member()
            .is_ok()
    );
    assert!(
        store
            .repositories()
            .repository_read_access("owner", "history", Some("history_owner"))
            .await
            .unwrap()
            .is_some()
    );
}
