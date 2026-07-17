use crate::{
    auth::scope::{optional_scope_user, principal_for_scope_user},
    domain::requests::RequestAudience,
    domain::store::{RepositoryActor, UserAccount, repo_id},
    error::ApiError,
    repo_events::{RepoChangeEvent, RepoChangeKind},
    state::{AppState, ensure_repo_read, find_repo},
};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::stream;
use std::{convert::Infallible, time::Duration};
use tokio_stream::{Stream, StreamExt, once, wrappers::BroadcastStream};

const CLIENT_RESYNC_VERSION: u64 = 9_007_199_254_740_991;

pub(crate) async fn repo_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;
    let repo_id = repo_id(&owner, &repo_name);
    let receiver = state.repo_events.subscribe(&repo_id);

    let repo = match find_repo(&state, &owner, &repo_name).await {
        Ok(repo) => repo,
        Err(error) => {
            drop(receiver);
            state.repo_events.remove_if_idle(&repo_id);
            return Err(error);
        }
    };
    let principal = principal_for_scope_user(&repo, user.as_ref());
    if let Err(error) = ensure_repo_events_allowed(&state, &repo, &principal) {
        drop(receiver);
        state.repo_events.remove_if_idle(&repo_id);
        return Err(error);
    }

    let initial = event_for_principal(
        &repo,
        &principal,
        RepoChangeEvent::new(&repo_id, repo.record.change_version, "connected"),
    )
    .expect("connected event is always visible");
    let updates = stream::unfold(
        RepoEventStreamState {
            lagged_repo_id: repo_id,
            owner,
            receiver: BroadcastStream::new(receiver),
            repo_name,
            state: state.clone(),
            user,
        },
        |mut stream_state| async move {
            loop {
                let event = stream_state.receiver.next().await?;
                let event = match event {
                    Ok(event) => event,
                    Err(_) => RepoChangeEvent {
                        repo_id: stream_state.lagged_repo_id.clone(),
                        version: CLIENT_RESYNC_VERSION,
                        kind: RepoChangeKind::Lagged,
                    },
                };

                match stream_event_for_user(
                    &stream_state.state,
                    &stream_state.owner,
                    &stream_state.repo_name,
                    stream_state.user.as_ref(),
                    event,
                )
                .await
                {
                    Ok(Some(event)) => return Some((sse_event(event), stream_state)),
                    Ok(None) => continue,
                    Err(_) => return None,
                }
            }
        },
    );

    Ok(
        Sse::new(once(sse_event(initial)).chain(updates)).keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(20))
                .text("keep-alive"),
        ),
    )
}

struct RepoEventStreamState {
    lagged_repo_id: String,
    owner: String,
    receiver: BroadcastStream<RepoChangeEvent>,
    repo_name: String,
    state: AppState,
    user: Option<UserAccount>,
}

async fn stream_event_for_user(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    user: Option<&UserAccount>,
    event: RepoChangeEvent,
) -> Result<Option<RepoChangeEvent>, ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    let principal = principal_for_scope_user(&repo, user);
    ensure_repo_events_allowed(state, &repo, &principal)?;
    Ok(event_for_principal(&repo, &principal, event))
}

fn ensure_repo_events_allowed(
    state: &AppState,
    repo: &crate::domain::store::StoredRepository,
    principal: &crate::domain::policy::Principal,
) -> Result<(), ApiError> {
    ensure_repo_read(state, repo, principal)
}

fn event_for_principal(
    repo: &crate::domain::store::StoredRepository,
    principal: &crate::domain::policy::Principal,
    event: RepoChangeEvent,
) -> Option<RepoChangeEvent> {
    if repo.access_for_principal(principal).actor != RepositoryActor::Public {
        return Some(event);
    }

    if let RepoChangeKind::RequestDiscussionChanged { audience, .. } = &event.kind {
        if matches!(audience, RequestAudience::Public) {
            return Some(RepoChangeEvent {
                version: 0,
                ..event
            });
        }
        return None;
    }

    Some(RepoChangeEvent {
        kind: match event.kind {
            RepoChangeKind::Connected => RepoChangeKind::Connected,
            RepoChangeKind::Lagged => RepoChangeKind::Lagged,
            _ => RepoChangeKind::RepositoryChanged {
                reason: "repo-changed".to_string(),
            },
        },
        repo_id: event.repo_id,
        version: 0,
    })
}

fn sse_event(event: RepoChangeEvent) -> Result<Event, Infallible> {
    let data = serde_json::to_string(&event).expect("repo change events must serialize");
    Ok(Event::default().event("repo-change").data(data))
}
