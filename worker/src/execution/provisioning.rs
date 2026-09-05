use anyhow::Context as _;
use std::future::Future;
use tokio::task::JoinSet;

// Bound AWS setup traffic separately from the number of running jobs.
const MAX_PROVIDER_STARTS: usize = 4;

pub(super) struct Provisioning {
    tasks: JoinSet<anyhow::Result<()>>,
    limit: usize,
}

impl Provisioning {
    pub(super) fn new(max_concurrency: usize) -> Self {
        Self {
            tasks: JoinSet::new(),
            limit: max_concurrency.clamp(1, MAX_PROVIDER_STARTS),
        }
    }

    // Wait before reserving another attempt, so a lease never sits in a local queue.
    pub(super) async fn wait_for_slot(&mut self) -> anyhow::Result<()> {
        if self.tasks.len() >= self.limit {
            self.tasks
                .join_next()
                .await
                .expect("a full provisioning set contains a task")
                .context("cloud provisioning task panicked")??;
        }
        Ok(())
    }

    pub(super) fn spawn(
        &mut self,
        provision: impl Future<Output = anyhow::Result<()>> + Send + 'static,
    ) {
        assert!(self.tasks.len() < self.limit);
        self.tasks.spawn(provision);
    }

    pub(super) async fn finish(mut self) -> anyhow::Result<()> {
        let mut first_error = None;
        while let Some(result) = self.tasks.join_next().await {
            if let Err(error) = result
                .context("cloud provisioning task panicked")
                .and_then(|r| r)
            {
                tracing::error!(error = %error, "cloud provisioning failed; durable attempt retains recovery ownership");
                first_error.get_or_insert(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
