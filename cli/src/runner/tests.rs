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
fn startup_cleanup_removes_abandoned_work_without_removing_root() {
    let root = env::temp_dir().join(format!("scope-runner-cleanup-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("attempt/workspace")).unwrap();
    fs::write(root.join("attempt/workspace/source.txt"), "private").unwrap();
    fs::write(root.join("orphan.bundle"), "bundle").unwrap();

    cleanup_work_root(&root).unwrap();

    assert!(root.is_dir());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    fs::remove_dir(root).unwrap();
}
