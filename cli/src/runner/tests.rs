use super::recovery::{
    mark_recovery_step_conclusion_pending, mark_recovery_step_started, stage_recovery_log_chunk,
    update_recovery_log_progress,
};
use super::*;
use scope_api_contract::StepConclusionRequest;
use scope_domain::runs::cache::WorkflowCache;
use scope_domain::runs::workflow::{
    CompiledWorkflow, ContainerSpec, RunnerSelector, WorkflowJob, WorkflowJobId, WorkflowStep,
    WorkflowTriggers,
};
use std::{env, fs};

#[test]
fn runner_client_sends_cli_compatibility_identity() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut bytes = [0_u8; 8192];
        let read = stream.read(&mut bytes).unwrap();
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .unwrap();
        String::from_utf8(bytes[..read].to_vec()).unwrap()
    });

    let response = runner_client()
        .unwrap()
        .get(format!("http://{address}/identity"))
        .send()
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    let request = server.join().unwrap().to_ascii_lowercase();
    assert!(request.contains("x-scope-cli-protocol: 1\r\n"));
    assert!(request.contains(&format!(
        "x-scope-cli-version: {}\r\n",
        crate::build::PACKAGE_VERSION
    )));
    assert!(request.contains(&format!(
        "x-scope-cli-build: {}\r\n",
        crate::build::BUILD_SHA
    )));
}

#[test]
fn repository_and_systemd_inputs_are_strict() {
    assert_eq!(parse_repository("owner/repo").unwrap(), ("owner", "repo"));
    assert!(parse_repository("owner").is_err());
    assert!(parse_repository("owner/repo/extra").is_err());
    assert_eq!(
        systemd_quote_path(Path::new("/opt/Scope Runner/%bin")).unwrap(),
        "\"/opt/Scope Runner/%%bin\""
    );
}

#[test]
fn docker_limits_are_always_applied() {
    let limits = ResourceLimits {
        memory_bytes: 4 * 1024 * 1024 * 1024,
        cpu_millis: 2000,
        pids: 512,
        storage_bytes: 20 * 1024 * 1024 * 1024,
    };
    let mut command = Command::new("docker");
    command.arg("run");
    apply_container_limits(&mut command, &limits, DockerCapabilities::default());
    let arguments = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        arguments,
        [
            "run",
            "--memory",
            "4294967296",
            "--memory-swap",
            "4294967296",
            "--cpus",
            "2.000",
            "--pids-limit",
            "512",
        ]
    );

    let mut quota_command = Command::new("docker");
    quota_command.arg("run");
    apply_container_limits(
        &mut quota_command,
        &limits,
        DockerCapabilities {
            storage_quota_supported: true,
        },
    );
    let quota_arguments = quota_command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        quota_arguments.ends_with(&["--storage-opt".to_string(), "size=21474836480".to_string()])
    );
}

#[test]
fn job_container_receives_only_copied_source_and_step_programs() {
    use scope_api_contract::RunJobResponse;

    let config = RunnerConfig {
        api_url: "https://api.example.test".to_string(),
        runner_id: "runner-1".to_string(),
        name: "linux-box".to_string(),
        secret: "runner-secret".to_string(),
        cache_root: None,
    };
    let claim = ClaimRunResponse {
        attempt_id: "attempt-1".to_string(),
        attempt_token: "attempt-secret".to_string(),
        lease_expires_at_unix: 100,
        canary_phase: None,
        job: RunJobResponse {
            run_id: "run-1".to_string(),
            repository_id: "owner/repo".to_string(),
            git_oid: "a".repeat(40),
            source_digest: "b".repeat(64),
            pinned_container_image: None,
            workflow: CompiledWorkflow::new(
                "Test",
                WorkflowTriggers::new(true, false).unwrap(),
                vec![WorkflowJob::new(
                    WorkflowJobId::parse("checks").unwrap(),
                    vec![], RunnerSelector::Any,
                    ContainerSpec::new("alpine:3.20").unwrap(), 60,
                    vec![WorkflowCache::parse("cargo").unwrap()],
                    vec![WorkflowStep::new("Test", "true").unwrap()],
                ).unwrap()],
            )
            .unwrap(),
        },
    };
    let mut command = Command::new("docker");
    let limits = ResourceLimits {
        memory_bytes: 1024,
        cpu_millis: 1000,
        pids: 128,
        storage_bytes: 2048,
    };
    let caches = [cache::CacheMount {
        volume_name: "scope-cache-v1-abc".to_string(),
        target: "/scope/cache/cargo".to_string(),
    }];
    configure_job_container_creation(
        &mut command,
        JobContainerSpec {
            config: &config,
            claim: &claim,
            name: "scope-attempt-1",
            image: "docker.io/library/alpine@sha256:abc",
            step_programs: Path::new("/runner/private/steps"),
            limits: &limits,
            capabilities: DockerCapabilities {
                storage_quota_supported: true,
            },
            caches: &caches,
        },
    );
    let arguments = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let joined = arguments.join(" ");

    assert_eq!(arguments[0], "create");
    for forbidden in [
        "--env",
        "-e",
        "-v",
        "/var/run/docker.sock",
        "DATABASE_URL",
        "SCOPE_",
        "--network=host",
    ] {
        assert!(!arguments.iter().any(|argument| argument == forbidden));
    }
    for forbidden in [
        "/var/run/docker.sock",
        "DATABASE_URL",
        "SCOPE_",
        "--network=host",
    ] {
        assert!(!joined.contains(forbidden));
    }
    assert!(!joined.contains(&config.secret));
    assert!(!joined.contains(&claim.attempt_token));
    assert!(joined.contains("scope.runner-id=runner-1"));
    assert!(joined.contains("scope.attempt-id=attempt-1"));
    assert!(joined.contains("type=bind,source=/runner/private/steps,target=/scope-steps,readonly"));
    assert!(joined.contains("type=volume,source=scope-cache-v1-abc,target=/scope/cache/cargo"));
    assert!(!joined.contains("--cgroup-parent"));
    assert!(!joined.contains("/scope-source"));
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--entrypoint", "sh"])
    );
    assert!(joined.contains("/scope-steps/current"));
    assert!(joined.contains("set -eu"));
    assert!(joined.contains("next_phase"));
    assert!(joined.contains("\"$next_phase\" = run"));
    assert!(joined.contains("sh -e"));
}

#[test]
fn log_encoding_is_chunk_independent_and_replay_safe() {
    let bytes = b"hello \xe2\x98\x83\nbad \xff\n";
    let whole = stable_log_text(bytes);
    let mut decoder = StableLogDecoder::default();
    let mut split = decoder.push(&bytes[..7]);
    split.push_str(&decoder.push(&bytes[7..10]));
    split.push_str(&decoder.push(&bytes[10..]));
    split.push_str(&decoder.finish());
    assert_eq!(split, whole);
    assert_eq!(whole, "hello ☃\nbad \\xff\n");
}

#[test]
fn replayed_step_conclusion_advances_local_recovery_to_the_next_step() {
    let mut progress = RecoveryProgress {
        next_log_sequence: 2,
        execution_deadline_unix: Some(90),
        active_step_index: Some(0),
        active_step_nonce: Some("nonce".to_string()),
        active_step_log_bytes: 4,
        logs_exhausted: false,
        pending_log_chunk: None,
        pending_step_conclusion: Some(recovery::PendingStepConclusion {
            step_index: 0,
            conclusion: StepConclusionRequest::Succeeded,
        }),
        pending_attempt_conclusion: None,
        pending_attempt_abandon: false,
        pending_cache_finalization: None,
        cache_volumes: Vec::new(),
    };

    advance_recovery_past_replayed_step(&mut progress);

    assert_eq!(progress.active_step_index, None);
    assert_eq!(progress.active_step_nonce, None);
    assert_eq!(progress.active_step_log_bytes, 0);
    assert_eq!(progress.pending_log_chunk, None);
    assert_eq!(progress.pending_step_conclusion, None);
}

#[cfg(unix)]
#[test]
fn interrupted_attempt_credentials_are_persisted_privately_for_reconciliation() {
    use scope_api_contract::RunJobResponse;
    use std::os::unix::fs::PermissionsExt;

    let root = env::temp_dir().join(format!("scope-runner-recovery-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    let claim = ClaimRunResponse {
        attempt_id: "attempt-1".to_string(),
        attempt_token: "secret-token".to_string(),
        lease_expires_at_unix: 100,
        canary_phase: None,
        job: RunJobResponse {
            run_id: "run-1".to_string(),
            repository_id: "owner/repo".to_string(),
            git_oid: "a".repeat(40),
            source_digest: "b".repeat(64),
            pinned_container_image: None,
            workflow: CompiledWorkflow::new(
                "Test",
                WorkflowTriggers::new(true, false).unwrap(),
                vec![WorkflowJob::new(
                    WorkflowJobId::parse("checks").unwrap(),
                    vec![], RunnerSelector::Any,
                    ContainerSpec::new("alpine:3.20").unwrap(), 60, Vec::new(),
                    vec![WorkflowStep::new("Test", "true").unwrap()],
                ).unwrap()],
            )
            .unwrap(),
        },
    };

    persist_recovery_claim(&root, &claim).unwrap();

    let path = root.join("claim.json");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!root.join(".claim.json.tmp").exists());
    assert!(persist_recovery_claim(&root, &claim).is_err());
    mark_recovery_step_started(&root, &claim, 0, "step-nonce").unwrap();
    stage_recovery_log_chunk(
        &root,
        &claim,
        recovery::PendingLogChunk {
            step_index: 0,
            sequence: 1,
            start_byte: 0,
            end_byte: 4,
            text: "test".to_string(),
        },
    )
    .unwrap();
    let staged: RecoveryProgress =
        serde_json::from_slice(&fs::read(root.join("progress.json")).unwrap()).unwrap();
    assert_eq!(
        staged.pending_log_chunk,
        Some(recovery::PendingLogChunk {
            step_index: 0,
            sequence: 1,
            start_byte: 0,
            end_byte: 4,
            text: "test".to_string(),
        })
    );
    update_recovery_log_progress(&root, &claim, 0, 2, 4, false).unwrap();
    mark_recovery_step_conclusion_pending(&root, &claim, 0, StepConclusionRequest::Succeeded)
        .unwrap();
    mark_recovery_execution_started(&root, &claim, 90).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(stored["schema_version"], 4);
    let stored: ClaimRunResponse = serde_json::from_value(stored["claim"].clone()).unwrap();
    assert_eq!(stored.attempt_id, claim.attempt_id);
    assert_eq!(stored.attempt_token, claim.attempt_token);
    let progress: RecoveryProgress =
        serde_json::from_slice(&fs::read(root.join("progress.json")).unwrap()).unwrap();
    assert_eq!(progress.next_log_sequence, 2);
    assert_eq!(progress.execution_deadline_unix, Some(90));
    assert_eq!(progress.active_step_index, Some(0));
    assert_eq!(progress.active_step_nonce.as_deref(), Some("step-nonce"));
    assert_eq!(progress.active_step_log_bytes, 4);
    assert_eq!(progress.pending_log_chunk, None);
    assert_eq!(
        progress.pending_step_conclusion,
        Some(recovery::PendingStepConclusion {
            step_index: 0,
            conclusion: StepConclusionRequest::Succeeded,
        })
    );
    assert!(!root.join(".progress.json.tmp").exists());
    fs::remove_dir_all(root).unwrap();
}
