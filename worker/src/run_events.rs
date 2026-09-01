use scope_api_contract::{RepoChangeEvent, RepoChangeKind, RepoChangeNotification, RunChangeKind};
use scope_postgres::db::MetadataStore;

pub(crate) async fn publish_run_change(
    metadata: &MetadataStore,
    origin_id: &str,
    repo_id: &str,
    run_id: &str,
    change: RunChangeKind,
) {
    let incarnation = match metadata
        .repositories()
        .run_repository_incarnation(run_id, repo_id)
        .await
    {
        Ok(Some(incarnation)) => incarnation,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(repo_id, run_id, error = %error.message, "failed to resolve run repository incarnation");
            return;
        }
    };
    let payload = match serde_json::to_string(&RepoChangeNotification {
        event: RepoChangeEvent {
            repo_id: repo_id.to_string(),
            incarnation_id: incarnation.incarnation_id().to_string(),
            version: 0,
            kind: RepoChangeKind::RunChanged {
                run_id: run_id.to_string(),
                change,
            },
        },
        origin_id: origin_id.to_string(),
    }) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(repo_id, run_id, %error, "failed to serialize run change notification");
            return;
        }
    };
    if let Err(error) = metadata.repositories().notify_repo_change(&payload).await {
        tracing::warn!(
            repo_id,
            run_id,
            error = %error.message,
            "failed to publish run change notification"
        );
    }
}
