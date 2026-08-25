use super::*;

impl ExecutionSink for RuntimeClient {
    fn start_step(&self, step: u32) -> anyhow::Result<bool> {
        Ok(RuntimeClient::start_step(self, step)?.cancellation_requested)
    }

    fn append_log(
        &self,
        step: u32,
        sequence: u64,
        text: &str,
    ) -> Result<AppendLogOutcome, AppendLogError> {
        RuntimeClient::append_log(self, step, sequence, text)
    }

    fn heartbeat(&self) -> anyhow::Result<bool> {
        Ok(RuntimeClient::heartbeat(self)?.cancellation_requested)
    }

    fn complete_step(&self, step: u32, exit_code: i32, logs_truncated: bool) -> anyhow::Result<()> {
        RuntimeClient::complete_step(self, step, exit_code, logs_truncated)?;
        Ok(())
    }

    fn complete_timeout(&self, logs_truncated: bool) -> anyhow::Result<()> {
        RuntimeClient::complete_timeout(self, logs_truncated)
    }

    fn complete_canceled(&self, logs_truncated: bool) -> anyhow::Result<()> {
        RuntimeClient::complete_canceled(self, logs_truncated)
    }

    fn abandon(&self) -> anyhow::Result<()> {
        RuntimeClient::abandon(self)
    }
}
