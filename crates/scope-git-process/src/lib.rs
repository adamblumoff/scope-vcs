mod lifecycle;
mod runner;
mod stdio;

pub use lifecycle::{
    ProcessSnapshot, configure_process_group, current_process_snapshot,
    install_pid1_reaper_if_needed, kill_process_group,
};
pub use runner::{
    ProcessCancellation, ProcessError, ProcessLimits, StreamedOutput, StreamingProcessError, run,
    run_with_stdin_reader, run_with_stdout,
};
pub use stdio::{STDERR_DIAGNOSTIC_BYTES, truncated_stderr};

#[cfg(test)]
mod tests;
