use super::request_discussions::{
    CONTRIBUTOR_ID as DEV_SEED_CONTRIBUTOR_ID, MAINTAINER_ID as DEV_SEED_MAINTAINER_ID,
    REQUEST_ID as DISCUSSION_REQUEST_ID,
};
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
    let submitted_blocks = catalog
        .request_change_blocks
        .values()
        .filter(|block| block.request_id == "req_demo_submitted")
        .collect::<Vec<_>>();
    assert_eq!(submitted_blocks.len(), 5);
    assert!(
        submitted_blocks
            .iter()
            .all(|block| block.git_snapshot.git_oid == block.new_head_oid)
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
