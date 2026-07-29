use super::*;
use scope_domain::runs::workflow::{
    CompiledWorkflow, ContainerSpec, RunnerSelector, WorkflowStep, WorkflowTriggers,
};
use std::io::Cursor;

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
fn job_script_preserves_order_and_quotes_step_labels() {
    let workflow = CompiledWorkflow::new(
        "Test",
        WorkflowTriggers::new(true, false).unwrap(),
        RunnerSelector::Any,
        ContainerSpec::new("alpine:3.20").unwrap(),
        60,
        vec![
            WorkflowStep::new("It's first", "printf one").unwrap(),
            WorkflowStep::new("Second", "printf two\n").unwrap(),
        ],
    )
    .unwrap();
    let script = job_script(&workflow);
    assert!(script.find("printf one").unwrap() < script.find("printf two").unwrap());
    assert!(script.contains("'It'\"'\"'s first'"));
    assert!(script.starts_with("#!/bin/sh\nset -e\n"));
}

#[test]
fn docker_limits_are_always_applied() {
    let mut command = Command::new("docker");
    command.arg("run");
    apply_container_limits(&mut command, false);
    let arguments = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        arguments,
        [
            "run",
            "--memory",
            "4g",
            "--memory-swap",
            "4g",
            "--cpus",
            "2",
            "--pids-limit",
            "512",
        ]
    );

    let mut quota_command = Command::new("docker");
    quota_command.arg("run");
    apply_container_limits(&mut quota_command, true);
    let quota_arguments = quota_command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(quota_arguments.ends_with(&["--storage-opt".to_string(), "size=20G".to_string()]));
}

#[test]
fn log_reader_bounds_chunks_even_without_newlines() {
    let input = vec![b'x'; LOG_CHUNK_BYTES * 2 + 7];
    let (sender, receiver) = mpsc::channel();
    let handle = spawn_log_reader(Cursor::new(input.clone()), sender);
    let chunks = receiver.into_iter().collect::<Vec<_>>();
    handle.join().unwrap();

    assert!(chunks.iter().all(|chunk| chunk.len() <= LOG_CHUNK_BYTES));
    assert_eq!(chunks.concat().into_bytes(), input);
}

#[test]
fn log_encoding_is_chunk_independent_and_replay_safe() {
    let bytes = b"hello \xe2\x98\x83\nbad \xff\n";
    let whole = stable_log_text(bytes);
    let split = [
        stable_log_text(&bytes[..7]),
        stable_log_text(&bytes[7..10]),
        stable_log_text(&bytes[10..]),
    ]
    .concat();
    assert_eq!(split, whole);
    assert_eq!(whole, "hello \\xe2\\x98\\x83\nbad \\xff\n");
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
        job: RunJobResponse {
            run_id: "run-1".to_string(),
            repository_id: "owner/repo".to_string(),
            git_oid: "a".repeat(40),
            source_digest: "b".repeat(64),
            pinned_container_image: None,
            workflow: CompiledWorkflow::new(
                "Test",
                WorkflowTriggers::new(true, false).unwrap(),
                RunnerSelector::Any,
                ContainerSpec::new("alpine:3.20").unwrap(),
                60,
                vec![WorkflowStep::new("Test", "true").unwrap()],
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
    update_recovery_log_sequence(&root, &claim, 2).unwrap();
    mark_recovery_execution_started(&root, &claim, 90).unwrap();
    let stored: ClaimRunResponse = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(stored.attempt_id, claim.attempt_id);
    assert_eq!(stored.attempt_token, claim.attempt_token);
    let progress: RecoveryProgress =
        serde_json::from_slice(&fs::read(root.join("progress.json")).unwrap()).unwrap();
    assert_eq!(progress.next_log_sequence, 2);
    assert_eq!(progress.execution_deadline_unix, Some(90));
    assert!(!root.join(".progress.json.tmp").exists());
    fs::remove_dir_all(root).unwrap();
}
