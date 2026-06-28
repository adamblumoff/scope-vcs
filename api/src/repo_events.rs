use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::broadcast;

const REPO_CHANGE_CHANNEL_CAPACITY: usize = 128;
pub(crate) const POSTGRES_REPO_CHANGE_CHANNEL: &str = "scope_repo_changes";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct RepoChangeEvent {
    pub(crate) repo_id: String,
    pub(crate) version: u64,
    pub(crate) reason: String,
}

impl RepoChangeEvent {
    pub(crate) fn new(repo_id: &str, version: u64, reason: &'static str) -> Self {
        Self {
            repo_id: repo_id.to_string(),
            version,
            reason: reason.to_string(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RepoChangeBus {
    channels: Arc<Mutex<BTreeMap<String, broadcast::Sender<RepoChangeEvent>>>>,
}

impl RepoChangeBus {
    pub(crate) fn start_postgres_listener(&self, database_url: String) -> anyhow::Result<()> {
        let bus = self.clone();
        std::thread::Builder::new()
            .name("scope-repo-change-listener".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        tracing::error!(%error, "failed to start repo change listener runtime");
                        return;
                    }
                };

                runtime.block_on(async move {
                    loop {
                        if let Err(error) =
                            listen_for_postgres_repo_changes(&database_url, &bus).await
                        {
                            tracing::warn!(%error, "repo change listener disconnected");
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                    }
                });
            })?;
        Ok(())
    }

    pub(crate) fn subscribe(&self, repo_id: &str) -> broadcast::Receiver<RepoChangeEvent> {
        let mut channels = self
            .channels
            .lock()
            .expect("repo change bus lock must not be poisoned");
        let sender = channels
            .entry(repo_id.to_string())
            .or_insert_with(|| broadcast::channel(REPO_CHANGE_CHANNEL_CAPACITY).0);
        sender.subscribe()
    }

    pub(crate) fn remove_if_idle(&self, repo_id: &str) {
        let mut channels = self
            .channels
            .lock()
            .expect("repo change bus lock must not be poisoned");
        if channels
            .get(repo_id)
            .is_some_and(|sender| sender.receiver_count() == 0)
        {
            channels.remove(repo_id);
        }
    }

    pub(crate) fn publish_event(&self, event: RepoChangeEvent) {
        let mut channels = self
            .channels
            .lock()
            .expect("repo change bus lock must not be poisoned");
        let Some(sender) = channels.get(&event.repo_id).cloned() else {
            return;
        };
        if sender.receiver_count() == 0 || sender.send(event.clone()).is_err() {
            channels.remove(&event.repo_id);
        }
    }
}

async fn listen_for_postgres_repo_changes(
    database_url: &str,
    bus: &RepoChangeBus,
) -> Result<(), sqlx::Error> {
    let mut listener = sqlx::postgres::PgListener::connect(database_url).await?;
    listener.listen(POSTGRES_REPO_CHANGE_CHANNEL).await?;
    loop {
        let notification = listener.recv().await?;
        match serde_json::from_str::<RepoChangeEvent>(notification.payload()) {
            Ok(event) => bus.publish_event(event),
            Err(error) => tracing::warn!(
                %error,
                payload = notification.payload(),
                "ignored malformed repo change notification"
            ),
        }
    }
}
