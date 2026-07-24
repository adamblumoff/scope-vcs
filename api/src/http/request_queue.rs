use super::{
    requests::{current_main_oid_for_access, repo_and_access},
    responses::request_list_item_response,
};
use crate::{
    domain::requests::{
        REQUEST_LIST_DEFAULT_PAGE_SIZE, REQUEST_LIST_MAX_PAGE_SIZE, RequestQueueSection,
    },
    error::ApiError,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use scope_api_contract::RequestListResponse;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const REQUEST_QUEUE_CURSOR_PREFIX: &str = "scope_rq_";
const REQUEST_QUEUE_CURSOR_NONCE_BYTES: usize = 12;
const REQUEST_QUEUE_CURSOR_MAX_ENCODED_BYTES: usize = 2_048;
const REQUEST_QUEUE_CURSOR_KEY_DOMAIN: &[u8] = b"scope.request-queue-cursor.key.v1\\0";
const REQUEST_QUEUE_CURSOR_AAD_DOMAIN: &str = "scope.request-queue-cursor.aad.v1";
const REQUEST_QUEUE_SEARCH_MAX_CHARS: usize = 200;

#[derive(Debug, Deserialize)]
pub(crate) struct RequestQueueQuery {
    section: RequestQueueSection,
    cursor: Option<String>,
    limit: Option<usize>,
    search: Option<String>,
}

pub(crate) async fn request_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<RequestQueueQuery>,
) -> Result<Json<RequestListResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let after = query
        .cursor
        .as_deref()
        .map(|cursor| {
            parse_request_queue_cursor(
                state.push_intent_signing_key.as_ref(),
                &repo.record.id,
                query.section,
                cursor,
            )
        })
        .transpose()?;
    let search = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|search| !search.is_empty());
    if search.is_some_and(|search| search.chars().count() > REQUEST_QUEUE_SEARCH_MAX_CHARS) {
        return Err(ApiError::bad_request("request queue search is too long"));
    }
    if query.section == RequestQueueSection::YourWork && search.is_some() {
        return Err(ApiError::bad_request(
            "search is only supported for ready and completed requests",
        ));
    }
    let limit = query
        .limit
        .unwrap_or(REQUEST_LIST_DEFAULT_PAGE_SIZE)
        .clamp(1, REQUEST_LIST_MAX_PAGE_SIZE);
    if query.section == RequestQueueSection::YourWork && viewer_user_id.is_none() {
        return Ok(Json(RequestListResponse {
            requests: Vec::new(),
            next_cursor: None,
        }));
    }

    let mut rows = state
        .metadata
        .request_queue_page(scope_core::db::RequestQueuePageQuery {
            repo_id: &repo.record.id,
            section: query.section,
            viewer_user_id: viewer_user_id.as_deref(),
            access,
            search,
            after: after.as_ref(),
            limit: (limit + 1) as u64,
        })
        .await?;
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = if has_more {
        rows.last()
            .map(|row| {
                encode_request_queue_cursor(
                    state.push_intent_signing_key.as_ref(),
                    &repo.record.id,
                    query.section,
                    &row.cursor,
                )
            })
            .transpose()?
    } else {
        None
    };
    let current_main_oid = current_main_oid_for_access(&state, &repo, access)?;
    let requests = rows
        .into_iter()
        .map(|row| request_list_item_response(row.request, access, current_main_oid.clone()))
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(Json(RequestListResponse {
        requests,
        next_cursor,
    }))
}

fn encode_request_queue_cursor(
    signing_key: &[u8],
    repo_id: &str,
    section: RequestQueueSection,
    cursor: &scope_core::db::RequestQueueCursor,
) -> Result<String, ApiError> {
    let plaintext = encode_request_queue_cursor_plain(cursor);
    let mut nonce = [0_u8; REQUEST_QUEUE_CURSOR_NONCE_BYTES];
    getrandom::fill(&mut nonce).map_err(|error| {
        ApiError::internal_message(format!("request queue cursor nonce failed: {error}"))
    })?;
    let key = request_queue_cursor_key(signing_key);
    let aad = request_queue_cursor_aad(repo_id, section);
    let ciphertext = ChaCha20Poly1305::new(Key::from_slice(&key))
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| ApiError::internal_message("request queue cursor encryption failed"))?;
    let mut envelope = Vec::with_capacity(nonce.len() + ciphertext.len());
    envelope.extend_from_slice(&nonce);
    envelope.extend_from_slice(&ciphertext);
    Ok(format!(
        "{REQUEST_QUEUE_CURSOR_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(envelope)
    ))
}

fn parse_request_queue_cursor(
    signing_key: &[u8],
    repo_id: &str,
    section: RequestQueueSection,
    value: &str,
) -> Result<scope_core::db::RequestQueueCursor, ApiError> {
    let invalid = || ApiError::bad_request("invalid request queue cursor");
    let encoded = value
        .strip_prefix(REQUEST_QUEUE_CURSOR_PREFIX)
        .filter(|encoded| encoded.len() <= REQUEST_QUEUE_CURSOR_MAX_ENCODED_BYTES)
        .ok_or_else(invalid)?;
    let envelope = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalid())?;
    if envelope.len() <= REQUEST_QUEUE_CURSOR_NONCE_BYTES {
        return Err(invalid());
    }
    let (nonce, ciphertext) = envelope.split_at(REQUEST_QUEUE_CURSOR_NONCE_BYTES);
    let key = request_queue_cursor_key(signing_key);
    let aad = request_queue_cursor_aad(repo_id, section);
    let plaintext = ChaCha20Poly1305::new(Key::from_slice(&key))
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| invalid())?;
    let plaintext = std::str::from_utf8(&plaintext).map_err(|_| invalid())?;
    parse_request_queue_cursor_plain(section, plaintext)
}

fn request_queue_cursor_key(signing_key: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(REQUEST_QUEUE_CURSOR_KEY_DOMAIN);
    digest.update(signing_key);
    digest.finalize().into()
}

fn request_queue_cursor_aad(repo_id: &str, section: RequestQueueSection) -> String {
    format!(
        "{REQUEST_QUEUE_CURSOR_AAD_DOMAIN}\0{repo_id}\0{}",
        request_queue_section_name(section)
    )
}

fn request_queue_section_name(section: RequestQueueSection) -> &'static str {
    match section {
        RequestQueueSection::YourWork => "your_work",
        RequestQueueSection::Ready => "ready",
        RequestQueueSection::Completed => "completed",
    }
}
fn parse_request_queue_cursor_plain(
    section: RequestQueueSection,
    value: &str,
) -> Result<scope_core::db::RequestQueueCursor, ApiError> {
    use scope_core::db::RequestQueueCursor;

    let parts = value.split(':').collect::<Vec<_>>();
    let invalid = || ApiError::bad_request("invalid request queue cursor");
    let request_id = |value: &str| {
        (!value.is_empty() && !value.contains(':'))
            .then(|| value.to_string())
            .ok_or_else(invalid)
    };
    let pg_u64 = |value: &str| -> Result<u64, ApiError> {
        let parsed = value.parse::<u64>().map_err(|_| invalid())?;
        i64::try_from(parsed).map_err(|_| invalid())?;
        Ok(parsed)
    };
    let pg_u32 = |value: &str| -> Result<u32, ApiError> {
        let parsed = value.parse::<u32>().map_err(|_| invalid())?;
        i32::try_from(parsed).map_err(|_| invalid())?;
        Ok(parsed)
    };
    match (section, parts.as_slice()) {
        (RequestQueueSection::YourWork, ["v1", "work", updated, id]) => {
            Ok(RequestQueueCursor::YourWork {
                updated_at_unix: pg_u64(updated)?,
                request_id: request_id(id)?,
            })
        }
        (RequestQueueSection::Ready, ["v1", "ready", snapshot, stake, ready, id]) => {
            Ok(RequestQueueCursor::Ready {
                snapshot_version: pg_u64(snapshot)?,
                stake_credits: pg_u32(stake)?,
                ready_at_unix: pg_u64(ready)?,
                request_id: request_id(id)?,
            })
        }
        (RequestQueueSection::Completed, ["v1", "completed", completed, id]) => {
            Ok(RequestQueueCursor::Completed {
                completed_at_unix: pg_u64(completed)?,
                request_id: request_id(id)?,
            })
        }
        _ => Err(invalid()),
    }
}

fn encode_request_queue_cursor_plain(cursor: &scope_core::db::RequestQueueCursor) -> String {
    use scope_core::db::RequestQueueCursor;

    match cursor {
        RequestQueueCursor::YourWork {
            updated_at_unix,
            request_id,
        } => format!("v1:work:{updated_at_unix}:{request_id}"),
        RequestQueueCursor::Ready {
            snapshot_version,
            stake_credits,
            ready_at_unix,
            request_id,
        } => format!("v1:ready:{snapshot_version}:{stake_credits}:{ready_at_unix}:{request_id}"),
        RequestQueueCursor::Completed {
            completed_at_unix,
            request_id,
        } => format!("v1:completed:{completed_at_unix}:{request_id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_core::db::RequestQueueCursor;

    #[test]
    fn queue_cursors_are_confidential_bound_and_range_checked() {
        const SIGNING_KEY: &[u8] = b"request-queue-test-signing-key";
        const REPO_ID: &str = "repo_test";
        for (section, cursor) in [
            (
                RequestQueueSection::YourWork,
                RequestQueueCursor::YourWork {
                    updated_at_unix: 10,
                    request_id: "req_work".to_string(),
                },
            ),
            (
                RequestQueueSection::Ready,
                RequestQueueCursor::Ready {
                    snapshot_version: 2,
                    stake_credits: 25,
                    ready_at_unix: 11,
                    request_id: "req_ready".to_string(),
                },
            ),
            (
                RequestQueueSection::Completed,
                RequestQueueCursor::Completed {
                    completed_at_unix: 12,
                    request_id: "req_completed".to_string(),
                },
            ),
        ] {
            let encoded =
                encode_request_queue_cursor(SIGNING_KEY, REPO_ID, section, &cursor).unwrap();
            let encoded_again =
                encode_request_queue_cursor(SIGNING_KEY, REPO_ID, section, &cursor).unwrap();
            assert!(encoded.starts_with(REQUEST_QUEUE_CURSOR_PREFIX));
            assert!(!encoded.contains("req_"));
            assert_ne!(encoded, encoded_again);
            assert_eq!(
                parse_request_queue_cursor(SIGNING_KEY, REPO_ID, section, &encoded).unwrap(),
                cursor
            );
            assert!(
                parse_request_queue_cursor(SIGNING_KEY, "repo_other", section, &encoded).is_err()
            );
        }

        let ready = RequestQueueCursor::Ready {
            snapshot_version: 2,
            stake_credits: 25,
            ready_at_unix: 11,
            request_id: "req_ready".to_string(),
        };
        let ready =
            encode_request_queue_cursor(SIGNING_KEY, REPO_ID, RequestQueueSection::Ready, &ready)
                .unwrap();
        assert!(
            parse_request_queue_cursor(
                SIGNING_KEY,
                REPO_ID,
                RequestQueueSection::Completed,
                &ready,
            )
            .is_err()
        );
        assert!(
            parse_request_queue_cursor_plain(
                RequestQueueSection::Completed,
                "v1:ready:2:25:11:req_ready"
            )
            .is_err()
        );
        for (section, cursor) in [
            (
                RequestQueueSection::YourWork,
                "v1:work:9223372036854775808:req",
            ),
            (
                RequestQueueSection::Ready,
                "v1:ready:9223372036854775808:1:1:req",
            ),
            (RequestQueueSection::Ready, "v1:ready:1:2147483648:1:req"),
            (
                RequestQueueSection::Ready,
                "v1:ready:1:1:9223372036854775808:req",
            ),
            (
                RequestQueueSection::Completed,
                "v1:completed:9223372036854775808:req",
            ),
        ] {
            assert!(parse_request_queue_cursor_plain(section, cursor).is_err());
        }
    }
}
