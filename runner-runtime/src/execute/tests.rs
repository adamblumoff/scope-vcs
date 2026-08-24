use super::supervisor::{SupervisorOptions, run_steps_with_options};
use super::{AppendLogError, AppendLogOutcome, ExecutionOutcome, ExecutionSink};
use anyhow::anyhow;
use scope_domain::runs::workflow::{ContainerSpec, WorkflowJob, WorkflowJobId, WorkflowStep};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const TEST_SYNC_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Start(u32),
    Append {
        step: u32,
        sequence: u64,
        text: String,
    },
    Heartbeat,
    CompleteStep {
        step: u32,
        exit_code: i32,
        logs_truncated: bool,
    },
    CompleteTimeout(bool),
    CompleteCanceled(bool),
    Abandon,
}

enum AppendAction {
    Accepted,
    Truncated,
    Retryable,
    Fatal,
    Gated {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        outcome: AppendLogOutcome,
    },
}

#[derive(Clone)]
struct FakeSink {
    state: Arc<FakeState>,
}

struct FakeState {
    calls: Mutex<Vec<Call>>,
    append_actions: Mutex<VecDeque<AppendAction>>,
    cancel_on_start: bool,
    cancel_on_heartbeat: AtomicBool,
    heartbeat_observed: Mutex<Option<mpsc::Sender<()>>>,
}

impl FakeSink {
    fn new(append_actions: impl IntoIterator<Item = AppendAction>) -> Self {
        Self {
            state: Arc::new(FakeState {
                calls: Mutex::new(Vec::new()),
                append_actions: Mutex::new(append_actions.into_iter().collect()),
                cancel_on_start: false,
                cancel_on_heartbeat: AtomicBool::new(false),
                heartbeat_observed: Mutex::new(None),
            }),
        }
    }

    fn observe_cancellation_heartbeat(self, observed: mpsc::Sender<()>) -> Self {
        *self.state.heartbeat_observed.lock().unwrap() = Some(observed);
        self
    }

    fn enable_heartbeat_cancellation(&self) {
        self.state
            .cancel_on_heartbeat
            .store(true, Ordering::Release);
    }

    fn calls(&self) -> Vec<Call> {
        self.state.calls.lock().unwrap().clone()
    }

    fn record(&self, call: Call) {
        self.state.calls.lock().unwrap().push(call);
    }
}

impl ExecutionSink for FakeSink {
    fn start_step(&self, step: u32) -> anyhow::Result<bool> {
        self.record(Call::Start(step));
        Ok(self.state.cancel_on_start)
    }

    fn append_log(
        &self,
        step: u32,
        sequence: u64,
        text: &str,
    ) -> Result<AppendLogOutcome, AppendLogError> {
        self.record(Call::Append {
            step,
            sequence,
            text: text.to_owned(),
        });
        let action = self
            .state
            .append_actions
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(AppendAction::Accepted);
        match action {
            AppendAction::Accepted => Ok(AppendLogOutcome::Accepted),
            AppendAction::Truncated => Ok(AppendLogOutcome::Truncated),
            AppendAction::Retryable => Err(AppendLogError::retryable(anyhow!("retry append"))),
            AppendAction::Fatal => Err(AppendLogError::fatal(anyhow!("fatal append"))),
            AppendAction::Gated {
                entered,
                release,
                outcome,
            } => {
                entered.send(()).unwrap();
                release.recv().unwrap();
                Ok(outcome)
            }
        }
    }

    fn heartbeat(&self) -> anyhow::Result<bool> {
        self.record(Call::Heartbeat);
        let cancellation_requested = self.state.cancel_on_heartbeat.load(Ordering::Acquire);
        if cancellation_requested
            && let Some(observed) = self.state.heartbeat_observed.lock().unwrap().as_ref()
        {
            let _ = observed.send(());
        }
        Ok(cancellation_requested)
    }

    fn complete_step(&self, step: u32, exit_code: i32, logs_truncated: bool) -> anyhow::Result<()> {
        self.record(Call::CompleteStep {
            step,
            exit_code,
            logs_truncated,
        });
        Ok(())
    }

    fn complete_timeout(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.record(Call::CompleteTimeout(logs_truncated));
        Ok(())
    }

    fn complete_canceled(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.record(Call::CompleteCanceled(logs_truncated));
        Ok(())
    }

    fn abandon(&self) -> anyhow::Result<()> {
        self.record(Call::Abandon);
        Ok(())
    }
}

#[test]
fn successful_steps_upload_bounded_chunks_with_global_sequences() {
    let sink = FakeSink::new([]);
    let workspace = tempfile::tempdir().unwrap();
    let job = job(&[
        ("first", "printf first; printf err >&2"),
        ("second", "head -c 20000 /dev/zero | tr '\\0' x"),
    ]);

    let outcome = run_steps_with_options(
        sink.clone(),
        &job,
        workspace.path(),
        SupervisorOptions::for_test(Duration::from_secs(2)),
    )
    .unwrap();

    assert_eq!(
        outcome,
        ExecutionOutcome::Succeeded {
            logs_truncated: false
        }
    );
    let calls = sink.calls();
    let appends = calls
        .iter()
        .filter_map(|call| match call {
            Call::Append {
                step,
                sequence,
                text,
            } => Some((*step, *sequence, text.len())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(appends.iter().any(|(step, _, _)| *step == 0));
    assert!(appends.iter().any(|(step, _, _)| *step == 1));
    assert_eq!(
        appends
            .iter()
            .map(|(_, sequence, _)| *sequence)
            .collect::<Vec<_>>(),
        (1..=appends.len() as u64).collect::<Vec<_>>()
    );
    assert!(appends.iter().all(|(_, _, bytes)| *bytes <= 64 * 1024));
    assert!(calls.contains(&Call::CompleteStep {
        step: 0,
        exit_code: 0,
        logs_truncated: false,
    }));
    assert!(calls.contains(&Call::CompleteStep {
        step: 1,
        exit_code: 0,
        logs_truncated: false,
    }));
}

#[test]
fn truncation_stops_uploads_but_completes_the_step_truthfully() {
    let sink = FakeSink::new([AppendAction::Truncated]);
    let workspace = tempfile::tempdir().unwrap();
    let job = job(&[("logs", "head -c 50000 /dev/zero | tr '\\0' x")]);

    let outcome = run_steps_with_options(
        sink.clone(),
        &job,
        workspace.path(),
        SupervisorOptions::for_test(Duration::from_secs(2)),
    )
    .unwrap();

    assert_eq!(
        outcome,
        ExecutionOutcome::Succeeded {
            logs_truncated: true
        }
    );
    let calls = sink.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, Call::Append { .. }))
            .count(),
        1
    );
    assert!(calls.contains(&Call::CompleteStep {
        step: 0,
        exit_code: 0,
        logs_truncated: true,
    }));
}

#[test]
fn nonzero_exit_drains_logs_before_completing_the_step() {
    let sink = FakeSink::new([]);
    let workspace = tempfile::tempdir().unwrap();
    let job = job(&[("fail", "printf failure >&2; exit 42")]);

    let outcome = run_steps_with_options(
        sink.clone(),
        &job,
        workspace.path(),
        SupervisorOptions::for_test(Duration::from_secs(2)),
    )
    .unwrap();

    assert_eq!(outcome, ExecutionOutcome::Terminal);
    let calls = sink.calls();
    let append_position = calls
        .iter()
        .position(|call| matches!(call, Call::Append { text, .. } if text == "failure"))
        .unwrap();
    let completion_position = calls
        .iter()
        .position(|call| {
            matches!(
                call,
                Call::CompleteStep {
                    step: 0,
                    exit_code: 42,
                    logs_truncated: false,
                }
            )
        })
        .unwrap();
    assert!(append_position < completion_position);
}

#[test]
fn output_can_close_before_the_process_exits() {
    let sink = FakeSink::new([]);
    let workspace = tempfile::tempdir().unwrap();
    let job = job(&[("close-output", "exec 1>&- 2>&-; sleep 0.15")]);

    let outcome = run_steps_with_options(
        sink.clone(),
        &job,
        workspace.path(),
        SupervisorOptions::for_test(Duration::from_secs(1)),
    )
    .unwrap();

    assert_eq!(
        outcome,
        ExecutionOutcome::Succeeded {
            logs_truncated: false
        }
    );
    assert!(sink.calls().contains(&Call::CompleteStep {
        step: 0,
        exit_code: 0,
        logs_truncated: false,
    }));
}

#[test]
fn escaped_descendant_cannot_hold_output_capture_open() {
    let sink = FakeSink::new([]);
    let workspace = tempfile::tempdir().unwrap();
    let job = job(&[(
        "escape",
        "setsid sh -c 'echo $$ > escaped.pid; sleep 30' & \
         while [ ! -s escaped.pid ]; do sleep 0.01; done",
    )]);
    let started = Instant::now();

    let outcome = run_steps_with_options(
        sink.clone(),
        &job,
        workspace.path(),
        SupervisorOptions::for_test(Duration::from_secs(2)),
    )
    .unwrap();
    let escaped_pid = std::fs::read_to_string(workspace.path().join("escaped.pid"))
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    unsafe {
        libc::kill(escaped_pid, libc::SIGKILL);
    }

    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        outcome,
        ExecutionOutcome::Succeeded {
            logs_truncated: false
        }
    );
    assert!(sink.calls().contains(&Call::CompleteStep {
        step: 0,
        exit_code: 0,
        logs_truncated: false,
    }));
}

#[test]
fn timeout_reaps_the_process_group_before_completing() {
    let sink = FakeSink::new([]);
    let workspace = tempfile::tempdir().unwrap();
    let job = job(&[("timeout", "sleep 30 & wait")]);
    let started = Instant::now();

    let outcome = run_steps_with_options(
        sink.clone(),
        &job,
        workspace.path(),
        SupervisorOptions::for_test(Duration::from_millis(40)),
    )
    .unwrap();

    assert_eq!(outcome, ExecutionOutcome::Terminal);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(sink.calls().contains(&Call::CompleteTimeout(false)));
}

#[test]
fn step_completion_waits_for_a_late_truncation_response() {
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let sink = FakeSink::new([AppendAction::Gated {
        entered: entered_sender,
        release: release_receiver,
        outcome: AppendLogOutcome::Truncated,
    }]);
    let workspace = tempfile::tempdir().unwrap();
    let job = job(&[("fast", "printf done")]);
    let run_sink = sink.clone();
    let run = thread::spawn(move || {
        run_steps_with_options(
            run_sink,
            &job,
            workspace.path(),
            SupervisorOptions::for_test(Duration::from_secs(2)),
        )
    });

    entered_receiver.recv_timeout(TEST_SYNC_TIMEOUT).unwrap();
    thread::sleep(Duration::from_millis(50));
    assert!(
        !sink
            .calls()
            .iter()
            .any(|call| matches!(call, Call::CompleteStep { .. }))
    );
    release_sender.send(()).unwrap();

    assert_eq!(
        run.join().unwrap().unwrap(),
        ExecutionOutcome::Succeeded {
            logs_truncated: true
        }
    );
    assert!(sink.calls().contains(&Call::CompleteStep {
        step: 0,
        exit_code: 0,
        logs_truncated: true,
    }));
}

#[test]
fn heartbeat_can_cancel_while_an_append_is_blocked() {
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let (heartbeat_sender, heartbeat_receiver) = mpsc::channel();
    let sink = FakeSink::new([AppendAction::Gated {
        entered: entered_sender,
        release: release_receiver,
        outcome: AppendLogOutcome::Accepted,
    }])
    .observe_cancellation_heartbeat(heartbeat_sender);
    let workspace = tempfile::tempdir().unwrap();
    let job = job(&[("slow", "head -c 50000 /dev/zero | tr '\\0' x; sleep 30")]);
    let run_sink = sink.clone();
    let run = thread::spawn(move || {
        run_steps_with_options(
            run_sink,
            &job,
            workspace.path(),
            SupervisorOptions::for_test(Duration::from_secs(10)),
        )
    });

    entered_receiver.recv_timeout(TEST_SYNC_TIMEOUT).unwrap();
    let started = Instant::now();
    sink.enable_heartbeat_cancellation();
    heartbeat_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    thread::sleep(Duration::from_millis(75));
    release_sender.send(()).unwrap();

    assert_eq!(run.join().unwrap().unwrap(), ExecutionOutcome::Terminal);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(sink.calls().contains(&Call::CompleteCanceled(true)));
}

#[test]
fn retry_reuses_the_exact_request_before_advancing_the_sequence() {
    let sink = FakeSink::new([
        AppendAction::Retryable,
        AppendAction::Accepted,
        AppendAction::Accepted,
    ]);
    let workspace = tempfile::tempdir().unwrap();
    let job = job(&[("retry", "head -c 20000 /dev/zero | tr '\\0' x")]);

    run_steps_with_options(
        sink.clone(),
        &job,
        workspace.path(),
        SupervisorOptions::for_test(Duration::from_secs(2)),
    )
    .unwrap();

    let appends = sink
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::Append { .. }))
        .collect::<Vec<_>>();
    assert!(appends.len() >= 3);
    assert_eq!(appends[0], appends[1]);
    assert!(matches!(
        (&appends[1], &appends[2]),
        (
            Call::Append { sequence: 1, .. },
            Call::Append { sequence: 2, .. }
        )
    ));
}

#[test]
fn fatal_upload_failure_reaps_the_process_group_then_abandons() {
    let sink = FakeSink::new([AppendAction::Fatal]);
    let workspace = tempfile::tempdir().unwrap();
    let job = job(&[("fail", "printf log; sleep 30 & wait")]);
    let started = Instant::now();

    let error = run_steps_with_options(
        sink.clone(),
        &job,
        workspace.path(),
        SupervisorOptions::for_test(Duration::from_secs(2)),
    )
    .unwrap_err();

    assert_eq!(error.to_string(), "fatal append");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(sink.calls().contains(&Call::Abandon));
    assert!(
        !sink
            .calls()
            .iter()
            .any(|call| matches!(call, Call::CompleteStep { .. }))
    );
}

fn job(steps: &[(&str, &str)]) -> WorkflowJob {
    WorkflowJob::new(
        WorkflowJobId::parse("test-job").unwrap(),
        Vec::new(),
        ContainerSpec::new(format!("test@sha256:{}", "0".repeat(64))).unwrap(),
        30,
        Vec::new(),
        Default::default(),
        steps
            .iter()
            .map(|(name, run)| WorkflowStep::new(*name, *run).unwrap())
            .collect(),
    )
    .unwrap()
}
