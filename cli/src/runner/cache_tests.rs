use super::*;
use crate::test_support::TestDir;
use std::os::unix::fs::MetadataExt;

fn record(state: CacheState) -> CacheRecord {
    record_for("runner-1", state)
}

fn record_for(runner_id: &str, state: CacheState) -> CacheRecord {
    let identity_digest = "a".repeat(64);
    let runner_namespace = runner_namespace(runner_id);
    CacheRecord {
        format: CACHE_FORMAT,
        runner_id: runner_id.to_string(),
        volume_name: volume_name(&runner_namespace, &identity_digest),
        runner_namespace,
        identity_digest,
        repository_id: "repo-1".to_string(),
        cache_name: "build".to_string(),
        image: "a".repeat(64),
        container_image: format!("example.test/build@sha256:{}", "a".repeat(64)),
        platform: "linux/amd64".to_string(),
        state,
        last_used_at_unix: 1,
    }
}

fn volume(record: &CacheRecord, backing: &Path) -> VolumeInspection {
    VolumeInspection {
        name: record.volume_name.clone(),
        driver: "local".to_string(),
        device: Some(backing.display().to_string()),
        volume_type: Some("none".to_string()),
        options: Some("bind".to_string()),
        labels: [
            ("scope.cache-format", "3"),
            ("scope.cache-key", record.identity_digest.as_str()),
            ("scope.repository-id", record.repository_id.as_str()),
            ("scope.cache-name", record.cache_name.as_str()),
            ("scope.image", record.image.as_str()),
            ("scope.platform", record.platform.as_str()),
            ("scope.runner-id", record.runner_id.as_str()),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect(),
    }
}

#[test]
fn physical_locations_are_stable_bounded_and_runner_namespaced() {
    let root = Path::new("/cache/root");
    let digest = "a".repeat(64);
    let first = CacheLocation::for_runner(root, "runner-1", &digest);
    let restarted = CacheLocation::for_runner(root, "runner-1", &digest);
    let colocated = CacheLocation::for_runner(root, "runner-2", &digest);

    assert_eq!(first, restarted);
    assert_ne!(first.volume_name, colocated.volume_name);
    assert_ne!(first.record_path, colocated.record_path);
    assert_ne!(first.backing_path, colocated.backing_path);
    assert!(first.volume_name.starts_with("scope-cache-v3-"));
    assert!(first.volume_name.len() < 64);
    assert_eq!(first.identity_digest, digest);
    assert_eq!(colocated.identity_digest, digest);
}

#[test]
fn records_keep_taint_explicit() {
    let state = CacheState::Tainted {
        attempt_id: "attempt-1".to_string(),
    };
    let json = serde_json::to_string(&state).unwrap();
    assert!(json.contains("tainted"));
    assert!(json.contains("attempt-1"));
}

#[test]
fn cache_cleanup_runs_as_root_inside_the_pinned_job_image() {
    let mut command = Command::new("docker");
    configure_backing_clear(
        &mut command,
        Path::new("/cache/data/identity"),
        "example.test/build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap();
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(args.windows(2).any(|pair| pair == ["--user", "0:0"]));
    assert!(
        args.windows(2)
            .any(|pair| { pair == ["--volume", "/cache/data/identity:/scope-cache"] })
    );
    assert!(args.iter().any(|arg| arg == "--network"));
    assert!(
        args.iter()
            .any(|arg| arg.starts_with("example.test/build@sha256:"))
    );
}

#[test]
fn only_ready_semantically_identical_metadata_is_warm() {
    let desired = record(CacheState::Tainted {
        attempt_id: "new-attempt".to_string(),
    });
    let ready = record(CacheState::Ready);
    assert!(metadata_allows_warm(&ready, &desired));

    let foreign_runner = record_for("runner-2", CacheState::Ready);
    assert!(!metadata_allows_warm(&foreign_runner, &desired));

    let tainted = record(CacheState::Tainted {
        attempt_id: "old-attempt".to_string(),
    });
    assert!(!metadata_allows_warm(&tainted, &desired));

    let mut wrong_image = ready;
    wrong_image.image = "sha256:different".to_string();
    assert!(!metadata_allows_warm(&wrong_image, &desired));
}

#[test]
fn physical_volume_must_match_backing_and_all_identity_labels() {
    let record = record(CacheState::Ready);
    let backing = Path::new("/cache/data/identity");
    let exact = volume(&record, backing);
    assert!(volume_matches(&exact, &record, backing, &record.runner_id));

    let mut wrong_backing = exact.clone();
    wrong_backing.device = Some("/cache/data/other".to_string());
    assert!(!volume_matches(
        &wrong_backing,
        &record,
        backing,
        &record.runner_id
    ));
    assert!(volume_is_owned(&wrong_backing, &record, &record.runner_id));

    let mut foreign = exact;
    foreign
        .labels
        .insert("scope.runner-id".to_string(), "runner-2".to_string());
    assert!(!volume_is_owned(&foreign, &record, "runner-1"));
}

#[test]
fn record_lookup_and_cleanup_candidates_are_runner_scoped() {
    let parent = TestDir::new("runner-cache-registration-isolation");
    let root = parent.path().join("scope/runner");
    initialize(&root).unwrap();
    let first = record_for("runner-1", CacheState::Ready);
    let second = record_for("runner-2", CacheState::Ready);
    write_record(&root, &first).unwrap();
    write_record(&root, &second).unwrap();

    assert_eq!(
        load_runner_records(&root, "runner-1").unwrap(),
        vec![first.clone()]
    );
    assert_eq!(
        load_runner_records(&root, "runner-2").unwrap(),
        vec![second.clone()]
    );
    assert!(read_record_for_volume(&root, &second.volume_name, "runner-1").is_err());
    assert_eq!(
        read_record_for_volume(&root, &first.volume_name, "runner-1").unwrap(),
        first.clone()
    );

    fs::write(
        record_location(&root, &second).record_path,
        b"foreign metadata is corrupt",
    )
    .unwrap();
    assert_eq!(load_runner_records(&root, "runner-1").unwrap(), vec![first]);
}

#[test]
fn ordinary_same_filesystem_directory_is_a_valid_cache_store() {
    let parent = TestDir::new("runner-cache-store");
    let root = parent.path().join("scope/runner");

    initialize(&root).unwrap();
    let capacity = validate_store(&root, false).unwrap();

    assert_eq!(
        fs::metadata(&root).unwrap().dev(),
        fs::metadata(parent.path()).unwrap().dev()
    );
    assert!(root.join("store.json").is_file());
    assert!(root.join("metadata").is_dir());
    assert!(root.join("data").is_dir());
    assert!(capacity.available_bytes > 0);
}

#[test]
fn cache_store_rejects_symlinks_and_noncanonical_paths() {
    let parent = TestDir::new("runner-cache-path-safety");
    let real = parent.path().join("real");
    fs::create_dir(&real).unwrap();
    let link = parent.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(validate_store(&link, true).is_err());

    let noncanonical = real.join("..").join("real");
    assert!(validate_store(&noncanonical, true).is_err());
}

#[test]
fn runner_cache_namespaces_reject_symlinks() {
    let parent = TestDir::new("runner-cache-namespace-symlinks");
    let root = parent.path().join("scope/runner");
    initialize(&root).unwrap();
    let record = record(CacheState::Ready);
    let location = record_location(&root, &record);

    let foreign_metadata = parent.path().join("foreign-metadata");
    fs::create_dir(&foreign_metadata).unwrap();
    std::os::unix::fs::symlink(
        &foreign_metadata,
        root.join("metadata").join(&record.runner_namespace),
    )
    .unwrap();
    assert!(write_record(&root, &record).is_err());
    fs::remove_file(root.join("metadata").join(&record.runner_namespace)).unwrap();

    let foreign_data = parent.path().join("foreign-data");
    fs::create_dir(&foreign_data).unwrap();
    std::os::unix::fs::symlink(
        &foreign_data,
        root.join("data").join(&record.runner_namespace),
    )
    .unwrap();
    assert!(create_backing_directory(&root, &location.backing_path).is_err());
}

#[test]
fn moving_an_initialized_cache_directory_does_not_brick_it() {
    let parent = TestDir::new("runner-cache-move");
    let original = parent.path().join("original");
    let moved = parent.path().join("moved");
    initialize(&original).unwrap();

    fs::rename(&original, &moved).unwrap();

    validate_store(&moved, false).unwrap();
}

#[test]
fn obsolete_cache_store_format_is_rejected() {
    let parent = TestDir::new("runner-cache-old-format");
    let root = parent.path().join("scope/runner");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("store.json"), br#"{"format":2}"#).unwrap();

    let error = validate_store(&root, false).unwrap_err();
    assert!(error.to_string().contains("schema is unsupported"));
}

#[test]
fn cleared_disposable_default_cache_is_reinitialized() {
    let parent = TestDir::new("runner-cache-disposable-default");
    let root = parent.path().join("scope/runner");
    initialize(&root).unwrap();
    fs::remove_file(root.join("store.json")).unwrap();
    fs::remove_dir(root.join("metadata")).unwrap();
    fs::remove_dir(root.join("data")).unwrap();
    assert!(root.join(".lifecycle.lock").is_file());
    ensure_usable_root(&root, true).unwrap();

    assert!(root.join("store.json").is_file());
    assert!(root.join("metadata").is_dir());
    assert!(root.join("data").is_dir());
}

#[test]
fn cleared_custom_cache_is_not_reinitialized() {
    let parent = TestDir::new("runner-cache-custom");
    let root = parent.path().join("custom");
    initialize(&root).unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert!(ensure_usable_root(&root, false).is_err());
    assert!(!root.exists());
}
