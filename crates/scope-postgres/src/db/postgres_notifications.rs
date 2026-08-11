use std::{sync::Arc, time::Duration};

const RECONNECT_DELAY: Duration = Duration::from_secs(2);
type PayloadHandler = Arc<dyn Fn(String) + Send + Sync>;

pub(super) fn start_listener(
    database_url: Option<&Arc<str>>,
    thread_name: &str,
    channel: &'static str,
    on_payload: impl Fn(String) + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let Some(database_url) = database_url else {
        return Ok(());
    };
    let database_url = database_url.to_string();
    let thread_name = thread_name.to_string();
    let on_payload: PayloadHandler = Arc::new(on_payload);
    std::thread::Builder::new()
        .name(thread_name.clone())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::error!(%error, %thread_name, "failed to start PostgreSQL listener runtime");
                    return;
                }
            };
            runtime.block_on(async move {
                loop {
                    let result = listen(&database_url, channel, Arc::clone(&on_payload)).await;
                    if let Err(error) = result {
                        tracing::warn!(%error, %channel, "PostgreSQL listener disconnected");
                        tokio::time::sleep(RECONNECT_DELAY).await;
                    }
                }
            });
        })?;
    Ok(())
}

async fn listen(
    database_url: &str,
    channel: &str,
    on_payload: PayloadHandler,
) -> Result<(), sqlx::Error> {
    let mut listener = sqlx::postgres::PgListener::connect(database_url).await?;
    listener.listen(channel).await?;
    loop {
        let notification = listener.recv().await?;
        on_payload(notification.payload().to_string());
    }
}
