use super::*;

fn record(state: CacheState) -> CacheRecord {
    let identity_digest = "a".repeat(64);
    CacheRecord {
        format: CACHE_FORMAT,
        volume_name: volume_name(&identity_digest),
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
            ("scope.cache-format", "1"),
            ("scope.cache-key", record.identity_digest.as_str()),
            ("scope.repository-id", record.repository_id.as_str()),
            ("scope.cache-name", record.cache_name.as_str()),
            ("scope.image", record.image.as_str()),
            ("scope.platform", record.platform.as_str()),
            ("scope.runner-id", "runner-1"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect(),
    }
}

#[test]
fn physical_names_are_bounded_and_versioned() {
    let digest = "a".repeat(64);
    let name = volume_name(&digest);
    assert_eq!(name, format!("scope-cache-v1-{}", "a".repeat(40)));
    assert!(name.len() < 64);
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
    assert!(volume_matches(&exact, &record, backing, "runner-1"));

    let mut wrong_backing = exact.clone();
    wrong_backing.device = Some("/cache/data/other".to_string());
    assert!(!volume_matches(
        &wrong_backing,
        &record,
        backing,
        "runner-1"
    ));
    assert!(volume_is_owned(&wrong_backing, &record, "runner-1"));

    let mut foreign = exact;
    foreign
        .labels
        .insert("scope.runner-id".to_string(), "runner-2".to_string());
    assert!(!volume_is_owned(&foreign, &record, "runner-1"));
}
