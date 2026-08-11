use super::MetadataStore;

const POSTGRES_RUN_DISPATCH_CHANNEL: &str = "scope_run_dispatch";

impl MetadataStore {
    pub fn start_run_dispatch_listener(
        &self,
        on_signal: impl Fn() + Send + Sync + 'static,
    ) -> anyhow::Result<()> {
        super::postgres_notifications::start_listener(
            self.postgres_database_url.as_ref(),
            "scope-run-dispatch-listener",
            POSTGRES_RUN_DISPATCH_CHANNEL,
            move |_| on_signal(),
        )
    }
}
