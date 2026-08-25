use super::*;

impl RuntimeClient {
    pub fn complete_step(
        &self,
        step: u32,
        exit_code: i32,
        logs_truncated: bool,
    ) -> anyhow::Result<AttemptStatusResponse> {
        let conclusion = if exit_code == 0 {
            StepConclusionRequest::Succeeded
        } else {
            StepConclusionRequest::Failed { exit_code }
        };
        self.post_json(
            &format!("steps/{step}/complete"),
            &CompleteAttemptStepRequest {
                conclusion,
                logs_truncated,
            },
            "complete step",
        )
    }

    pub fn complete_timeout(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::TimedOut, logs_truncated)
    }

    pub fn complete_succeeded(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::Succeeded, logs_truncated)
    }

    pub fn complete_canceled(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::Canceled, logs_truncated)
    }

    pub fn complete_setup_failure(&self, message: &str) -> anyhow::Result<()> {
        if self
            .attempt_token
            .lock()
            .expect("attempt token mutex poisoned")
            .is_none()
        {
            return Ok(());
        }
        self.complete(
            AttemptConclusionRequest::SetupFailed {
                exit_code: 70,
                message: message.chars().take(2048).collect(),
            },
            false,
        )
    }

    fn complete(
        &self,
        conclusion: AttemptConclusionRequest,
        logs_truncated: bool,
    ) -> anyhow::Result<()> {
        let _: AttemptStatusResponse = self.post_json(
            "complete",
            &CompleteAttemptRequest {
                conclusion,
                logs_truncated,
            },
            "complete attempt",
        )?;
        Ok(())
    }

    pub fn abandon(&self) -> anyhow::Result<()> {
        let response = self
            .auth(self.client.post(self.url("abandon")))
            .send()
            .context("abandon attempt")?;
        ensure_success(&response, "abandon attempt")
    }
}
