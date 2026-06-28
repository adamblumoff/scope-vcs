use crate::{
    auth::scope::{optional_scope_user, principal_for_scope_user},
    domain::store::{RepoRole, UserAccount, repo_id},
    error::ApiError,
    repo_events::RepoChangeEvent,
    state::{AppState, ensure_repo_read, find_repo, role_for_principal},
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

    let repo = match find_repo(&state, &owner, &repo_name) {
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
        &state,
        &repo,
        &principal,
        RepoChangeEvent::new(&repo_id, repo.record.change_version, "connected"),
    )?;
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
            let event = stream_state.receiver.next().await?;
            let event = match event {
                Ok(event) => event,
                Err(_) => RepoChangeEvent {
                    repo_id: stream_state.lagged_repo_id.clone(),
                    version: CLIENT_RESYNC_VERSION,
                    reason: "lagged".to_string(),
                },
            };

            let event = match stream_event_for_user(
                &stream_state.state,
                &stream_state.owner,
                &stream_state.repo_name,
                stream_state.user.as_ref(),
                event,
            ) {
                Ok(event) => event,
                Err(_) => return None,
            };

            Some((sse_event(event), stream_state))
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

fn stream_event_for_user(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    user: Option<&UserAccount>,
    event: RepoChangeEvent,
) -> Result<RepoChangeEvent, ApiError> {
    let repo = find_repo(state, owner, repo_name)?;
    let principal = principal_for_scope_user(&repo, user);
    ensure_repo_events_allowed(state, &repo, &principal)?;
    event_for_principal(state, &repo, &principal, event)
}

fn ensure_repo_events_allowed(
    state: &AppState,
    repo: &crate::domain::store::StoredRepository,
    principal: &crate::domain::policy::Principal,
) -> Result<(), ApiError> {
    ensure_repo_read(state, repo, principal)
}

fn event_for_principal(
    state: &AppState,
    repo: &crate::domain::store::StoredRepository,
    principal: &crate::domain::policy::Principal,
    event: RepoChangeEvent,
) -> Result<RepoChangeEvent, ApiError> {
    if role_for_principal(state, repo, principal)?.is_some_and(|role| role >= RepoRole::Writer) {
        return Ok(event);
    }

    let RepoChangeEvent {
        reason, repo_id, ..
    } = event;
    let reason = match reason.as_str() {
        "connected" | "lagged" => reason,
        _ => "repo-changed".to_string(),
    };

    Ok(RepoChangeEvent {
        reason,
        repo_id,
        version: 0,
    })
}

fn sse_event(event: RepoChangeEvent) -> Result<Event, Infallible> {
    let data = serde_json::to_string(&event).expect("repo change events must serialize");
    Ok(Event::default().event("repo-change").data(data))
}
