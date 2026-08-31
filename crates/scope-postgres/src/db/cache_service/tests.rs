use super::*;
use crate::db::{CatalogFixture, MetadataStore, TestDatabaseTarget};
use scope_domain::{
    account::UserAccount,
    policy::Visibility,
    repository::{RepoLifecycleState, Repository},
};

#[test]
fn object_keys_are_repository_scoped_and_content_addressed() {
    assert_eq!(
        cache_object_key("repo-1", &"a".repeat(64)),
        format!("repos/repo-1/objects/sha256/{}", "a".repeat(64))
    );
}

#[tokio::test]
async fn cache_store_restores_exact_then_compatible_and_never_repoints_exact() {
    let target = TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    let repository_id = seed_repository(&store);
    let caches = store.caches();
    let now = 1_700_000_000_u64;
    let identity = "1".repeat(64);
    let compatible_identity = "2".repeat(64);
    let compatibility_group = "f".repeat(64);
    let first_digest = "a".repeat(64);
    let second_digest = "b".repeat(64);

    let CachePrepareResult::Upload(_) = caches
        .prepare_upload(
            &repository_id,
            &identity,
            &compatibility_group,
            &first_digest,
            100,
            "test-local",
            "upload-1",
            now,
        )
        .await
        .unwrap()
    else {
        panic!("first content must require an upload");
    };
    assert!(matches!(
        caches.commit_upload("upload-1", now + 1).await.unwrap(),
        CacheCommitResult::Committed { .. }
    ));
    assert!(matches!(
        caches.commit_upload("upload-1", now + 1).await.unwrap(),
        CacheCommitResult::AlreadyCommitted { .. }
    ));
    let exact = caches
        .restore(&repository_id, &identity, &compatibility_group, now + 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exact.source, CacheRestoreKind::Exact);
    assert_eq!(exact.object.checksum_sha256, first_digest);

    let group_mismatch = caches
        .prepare_upload(
            &repository_id,
            &identity,
            &"d".repeat(64),
            &first_digest,
            100,
            "test-local",
            "mismatched-group-upload",
            now + 3,
        )
        .await
        .unwrap_err();
    assert_eq!(
        group_mismatch.kind,
        crate::error::PostgresErrorKind::Conflict
    );

    caches
        .db
        .execute(statement(
            "UPDATE scope_cache_references SET compatibility_group_digest = $1
             WHERE repository_id = $2 AND identity_digest = $3",
            vec![
                "d".repeat(64).into(),
                repository_id.clone().into(),
                identity.clone().into(),
            ],
        ))
        .await
        .unwrap();
    assert!(
        caches
            .restore(&repository_id, &identity, &compatibility_group, now + 3)
            .await
            .unwrap()
            .is_none(),
        "an exact identity row outside the authorized group must not be restored"
    );
    caches
        .db
        .execute(statement(
            "UPDATE scope_cache_references SET compatibility_group_digest = $1
             WHERE repository_id = $2 AND identity_digest = $3",
            vec![
                compatibility_group.clone().into(),
                repository_id.clone().into(),
                identity.clone().into(),
            ],
        ))
        .await
        .unwrap();

    let compatible = caches
        .restore(
            &repository_id,
            &compatible_identity,
            &compatibility_group,
            now + 4,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(compatible.source, CacheRestoreKind::Compatible);
    assert_eq!(compatible.object.checksum_sha256, first_digest);

    assert!(matches!(
        caches
            .prepare_upload(
                &repository_id,
                &compatible_identity,
                &compatibility_group,
                &first_digest,
                100,
                "test-local",
                "unused-upload",
                now + 5,
            )
            .await
            .unwrap(),
        CachePrepareResult::UseObject { .. }
    ));

    let exact_after_publish = caches
        .restore(
            &repository_id,
            &compatible_identity,
            &compatibility_group,
            now + 6,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exact_after_publish.source, CacheRestoreKind::Exact);

    assert!(matches!(
        caches
            .prepare_upload(
                &repository_id,
                &identity,
                &compatibility_group,
                &second_digest,
                200,
                "test-local",
                "upload-2",
                now + 7,
            )
            .await
            .unwrap(),
        CachePrepareResult::UseObject { ref object, .. }
            if object.checksum_sha256 == first_digest
    ));

    let expired_identity = "3".repeat(64);
    let expired_digest = "c".repeat(64);
    caches
        .prepare_upload(
            &repository_id,
            &expired_identity,
            &"e".repeat(64),
            &expired_digest,
            300,
            "test-local",
            "expired-upload",
            now + 10,
        )
        .await
        .unwrap();
    let cleanup_now = now + 10 + CachePolicy.upload_lease_seconds();
    let expired = caches.expire_uploads(cleanup_now, 10).await.unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].upload_id, "expired-upload");
    assert!(
        caches
            .expire_uploads(cleanup_now, 10)
            .await
            .unwrap()
            .is_empty()
    );
    caches.retry_upload_cleanup("expired-upload").await.unwrap();
    assert_eq!(
        caches.expire_uploads(cleanup_now, 10).await.unwrap().len(),
        1
    );
    caches
        .complete_upload_cleanup("expired-upload")
        .await
        .unwrap();
    assert!(
        caches
            .expire_uploads(cleanup_now, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

fn seed_repository(store: &MetadataStore) -> String {
    let owner = UserAccount {
        id: "user_cache_owner".to_string(),
        handle: "cache-owner".to_string(),
        email: "cache-owner@example.com".to_string(),
        email_verified: true,
    };
    let mut repository = Repository::new(&owner, "cache-repo", Visibility::Private, "repoi_test")
        .expect("test repository is valid");
    repository.record.lifecycle_state = RepoLifecycleState::Ready;
    let repository_id = repository.record.id.clone();
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog
        .repositories
        .insert(repository_id.clone(), repository);
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    repository_id
}
