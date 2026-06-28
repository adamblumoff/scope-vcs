use crate::{
    auth::scope::{optional_scope_user, principal_for_scope_user},
    error::ApiError,
    repo_events::RepoChangeEvent,
    state::{AppState, ensure_repo_read, find_repo},
};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use std::{convert::Infallible, time::Duration};
use tokio_stream::{Stream, StreamExt, once, wrappers::BroadcastStream};

const CLIENT_RESYNC_VERSION: u64 = 9_007_199_254_740_991;

pub(crate) async fn repo_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let user = optional_scope_user(&state, &headers).await?;
    let principal = principal_for_scope_user(&repo, user.as_ref());
    ensure_repo_read(&state, &repo, &principal)?;

    let repo_id = repo.record.id.clone();
    let (initial, receiver) = state
        .repo_events
        .subscribe(&repo_id, repo.record.change_version);
    let lagged_repo_id = repo_id.clone();
    let updates = BroadcastStream::new(receiver).map(move |event| match event {
        Ok(event) => sse_event(event),
        Err(_) => sse_event(RepoChangeEvent {
            repo_id: lagged_repo_id.clone(),
            version: CLIENT_RESYNC_VERSION,
            reason: "lagged".to_string(),
        }),
    });

    Ok(
        Sse::new(once(sse_event(initial)).chain(updates)).keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(20))
                .text("keep-alive"),
        ),
    )
}

fn sse_event(event: RepoChangeEvent) -> Result<Event, Infallible> {
    let data = serde_json::to_string(&event).expect("repo change events must serialize");
    Ok(Event::default().event("repo-change").data(data))
}
