use crate::{
    auth::scope::require_scope_user,
    error::ApiError,
    http::run_response::run_response,
    repo_events::{RepoChangeEvent, RepoChangeKind},
    state::AppState,
    use_cases::run_inspection::require_run_access,
};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use scope_api_contract::{ErrorResponse, RunEventsQuery, RunLogResponse};
use std::{
    convert::Infallible,
    time::{Duration, Instant},
};
use tokio_stream::wrappers::ReceiverStream;

const RUN_LOG_STREAM_PAGE_SIZE: usize = 64;
const RUN_LOG_STREAM_BUFFER: usize = 32;
const RUN_LOG_STREAM_AUTH_RECHECK: Duration = Duration::from_secs(30);
const RUN_LOG_STREAM_RECONCILE_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) async fn run_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
    Query(query): Query<RunEventsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let run = require_run_access(&state, &user.id, &owner, &repo_name, &run_id).await?;
    let run_changes = state.repo_events.subscribe(run.workflow.repository_id());
    let after = match headers.get("last-event-id") {
        Some(value) => value
            .to_str()
            .map_err(|_| ApiError::bad_request("last-event-id must be an integer"))?
            .parse::<u64>()
            .map_err(|_| ApiError::bad_request("last-event-id must be an integer"))?
            .max(query.after),
        None => query.after,
    };
    let (sender, receiver) = tokio::sync::mpsc::channel(RUN_LOG_STREAM_BUFFER);
    tokio::spawn(stream_run_events(
        RunEventStreamContext {
            state,
            headers,
            owner,
            repo_name,
            user_id: user.id,
            run_id,
            run_changes,
        },
        after,
        sender,
    ));
    Ok(Sse::new(ReceiverStream::new(receiver)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keep-alive"),
    ))
}

struct RunEventStreamContext {
    state: AppState,
    headers: HeaderMap,
    owner: String,
    repo_name: String,
    user_id: String,
    run_id: String,
    run_changes: tokio::sync::broadcast::Receiver<RepoChangeEvent>,
}

async fn stream_run_events(
    mut context: RunEventStreamContext,
    mut cursor: u64,
    sender: tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
) {
    let mut last_state = None;
    let mut terminal_observed = false;
    let mut authenticated_at = Instant::now();
    loop {
        if sender.is_closed() {
            return;
        }
        if authenticated_at.elapsed() >= RUN_LOG_STREAM_AUTH_RECHECK {
            match require_scope_user(&context.state, &context.headers).await {
                Ok(user) if user.id == context.user_id => authenticated_at = Instant::now(),
                Ok(_) => {
                    send_stream_error(&sender, ApiError::forbidden("run access changed")).await;
                    return;
                }
                Err(error) => {
                    send_stream_error(&sender, error).await;
                    return;
                }
            }
        }
        if let Err(error) = require_run_access(
            &context.state,
            &context.user_id,
            &context.owner,
            &context.repo_name,
            &context.run_id,
        )
        .await
        {
            send_stream_error(&sender, error).await;
            return;
        }
        let logs = match context
            .state
            .metadata
            .runs()
            .run_logs_after(&context.run_id, cursor, RUN_LOG_STREAM_PAGE_SIZE as u64)
            .await
        {
            Ok(logs) => logs,
            Err(error) => {
                send_stream_error(&sender, error.into()).await;
                return;
            }
        };
        let has_full_page = logs.len() == RUN_LOG_STREAM_PAGE_SIZE;
        for log in logs {
            cursor = log.position;
            let response = RunLogResponse {
                attempt_id: log.chunk.attempt_id,
                job_key: log.job_key,
                step_index: log.chunk.step_index,
                position: log.position,
                sequence: log.chunk.sequence,
                text: log.chunk.text,
                created_at_unix: log.chunk.created_at_unix,
            };
            let event = match Event::default()
                .event("log")
                .id(cursor.to_string())
                .json_data(response)
            {
                Ok(event) => event,
                Err(error) => {
                    send_stream_error(&sender, ApiError::internal(error)).await;
                    return;
                }
            };
            if sender.send(Ok(event)).await.is_err() {
                return;
            }
        }

        let snapshot = match context
            .state
            .metadata
            .runs()
            .run_snapshot(&context.run_id)
            .await
        {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                send_stream_error(&sender, ApiError::not_found("run no longer exists")).await;
                return;
            }
            Err(error) => {
                send_stream_error(&sender, error.into()).await;
                return;
            }
        };
        let run = &snapshot.run;
        let terminal = run.state.is_terminal();
        if terminal && !terminal_observed {
            // Log append and attempt completion are separate transactions. Seeing the terminal
            // state makes completion a stable watermark because terminal attempts reject later
            // logs; read once more before closing so a log committed between the two queries
            // above cannot be omitted.
            terminal_observed = true;
            continue;
        }
        if last_state != Some(run.state) && (!terminal || !has_full_page) {
            last_state = Some(run.state);
            let response = match run_response(run, &snapshot.jobs, snapshot.logs_truncated) {
                Ok(response) => response,
                Err(error) => {
                    send_stream_error(&sender, error).await;
                    return;
                }
            };
            let event = match Event::default().event("status").json_data(response) {
                Ok(event) => event,
                Err(error) => {
                    send_stream_error(&sender, ApiError::internal(error)).await;
                    return;
                }
            };
            if sender.send(Ok(event)).await.is_err() {
                return;
            }
        }
        if terminal && !has_full_page {
            return;
        }
        if has_full_page {
            continue;
        }
        tokio::select! {
            () = sender.closed() => return,
            () = wait_for_run_change(&mut context.run_changes, &context.run_id) => {}
            () = tokio::time::sleep(RUN_LOG_STREAM_RECONCILE_INTERVAL) => {}
        }
    }
}

async fn wait_for_run_change(
    receiver: &mut tokio::sync::broadcast::Receiver<RepoChangeEvent>,
    run_id: &str,
) {
    loop {
        match receiver.recv().await {
            Ok(event) => match event.kind {
                RepoChangeKind::RunChanged {
                    run_id: changed_run_id,
                    ..
                } if changed_run_id == run_id => return,
                RepoChangeKind::RepositoryChanged { .. } => return,
                _ => {}
            },
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => return,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                std::future::pending::<()>().await
            }
        }
    }
}

async fn send_stream_error(
    sender: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    error: ApiError,
) {
    let data = stream_error_data(error);
    if let Ok(event) = Event::default().event("error").json_data(data) {
        let _ = sender.send(Ok(event)).await;
    }
}

fn stream_error_data(error: ApiError) -> ErrorResponse {
    error.into_public_parts().1
}
#[cfg(test)]
mod tests {
    use super::*;
    use scope_api_contract::RunChangeKind;

    #[test]
    fn stream_errors_redact_database_diagnostics() {
        let diagnostic = "relation scope_runs_internal does not exist";
        let error = scope_postgres::error::PostgresError::internal_message(diagnostic);

        let data = stream_error_data(error.into());

        assert_eq!(data.code, scope_api_contract::ErrorCode::Internal);
        assert_eq!(data.message, "Scope hit an internal error.");
        assert!(!data.message.contains(diagnostic));
        assert!(data.error_reference.is_some());
        assert!(!data.retryable);
    }

    #[tokio::test]
    async fn run_stream_wakes_only_for_its_run_or_repository_changes() {
        let (sender, mut receiver) = tokio::sync::broadcast::channel(4);
        sender
            .send(RepoChangeEvent {
                repo_id: "owner/repo".to_string(),
                incarnation_id: "repoi_test".to_string(),
                version: 0,
                kind: RepoChangeKind::RunChanged {
                    run_id: "another-run".to_string(),
                    change: RunChangeKind::LogsAppended,
                },
            })
            .unwrap();
        sender
            .send(RepoChangeEvent {
                repo_id: "owner/repo".to_string(),
                incarnation_id: "repoi_test".to_string(),
                version: 0,
                kind: RepoChangeKind::RunChanged {
                    run_id: "target-run".to_string(),
                    change: RunChangeKind::StatusChanged,
                },
            })
            .unwrap();

        tokio::time::timeout(
            Duration::from_millis(100),
            wait_for_run_change(&mut receiver, "target-run"),
        )
        .await
        .unwrap();

        let (sender, mut receiver) = tokio::sync::broadcast::channel(1);
        sender
            .send(RepoChangeEvent {
                repo_id: "owner/repo".to_string(),
                incarnation_id: "repoi_test".to_string(),
                version: 4,
                kind: RepoChangeKind::RepositoryChanged {
                    reason: "member-removed".to_string(),
                },
            })
            .unwrap();
        tokio::time::timeout(
            Duration::from_millis(100),
            wait_for_run_change(&mut receiver, "target-run"),
        )
        .await
        .unwrap();
    }
}
