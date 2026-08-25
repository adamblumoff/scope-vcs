use super::requests::{random_id, repo_and_access, visible_request};
use crate::{
    auth::scope::require_scope_user, error::ApiError, persistence::unix_now,
    product_analytics::ProductEvent, repo_events::RepoChangeReason, state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_api_contract::{
    CreateRequestRatingRequest, RequestRatingParticipantResponse, RequestRatingResponse,
    RequestRatingsResponse,
};
use scope_domain::{
    account::UserAccount,
    requests::{CreateRequestRatingInput, RequestRating, eligible_rating_subject_user_id},
};
use std::collections::BTreeMap;

pub(crate) async fn list_request_ratings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestRatingsResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(
        &state,
        &repo,
        access,
        viewer_user_id.as_deref(),
        &request_id,
    )
    .await?;
    ratings_response(&state, &request, viewer_user_id.as_deref()).await
}

pub(crate) async fn create_request_rating(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(payload): Json<CreateRequestRatingRequest>,
) -> Result<Json<RequestRatingResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(&state, &repo, access, Some(&user.id), &request_id).await?;
    let rating = state
        .metadata
        .requests()
        .create_request_rating(CreateRequestRatingInput {
            id: random_id("request_rating")?,
            request_id: request.id.clone(),
            actor_user_id: user.id.clone(),
            score: payload.score,
            reason: payload.reason,
            now_unix: unix_now()?,
        })
        .await?;
    state.product_analytics.capture(ProductEvent::request_rated(
        &user.id,
        request.audience,
        rating.score,
    ));
    state
        .publish_request_summary_refresh(&repo.record.id, RepoChangeReason::RequestRated)
        .await;
    let users = state
        .metadata
        .requests()
        .users_by_ids([rating.rater_user_id.clone(), rating.subject_user_id.clone()])
        .await?;
    let participants = rating_participants(&state, &users).await?;
    Ok(Json(rating_response(rating, &participants)?))
}

async fn ratings_response(
    state: &AppState,
    request: &scope_domain::requests::Request,
    viewer_user_id: Option<&str>,
) -> Result<Json<RequestRatingsResponse>, ApiError> {
    let ratings = state
        .metadata
        .requests()
        .request_ratings(&request.id)
        .await?;
    let eligible_subject_id = viewer_user_id.and_then(|viewer_user_id| {
        eligible_rating_subject_user_id(request, viewer_user_id, &ratings).map(str::to_string)
    });
    let user_ids = ratings
        .iter()
        .flat_map(|rating| [rating.rater_user_id.clone(), rating.subject_user_id.clone()])
        .chain(eligible_subject_id.iter().cloned());
    let users = state.metadata.requests().users_by_ids(user_ids).await?;
    let participants = rating_participants(state, &users).await?;
    let ratings = ratings
        .into_iter()
        .map(|rating| rating_response(rating, &participants))
        .collect::<Result<Vec<_>, _>>()?;
    let eligible_subject = eligible_subject_id
        .as_deref()
        .map(|user_id| participant_response(user_id, &participants))
        .transpose()?;
    Ok(Json(RequestRatingsResponse {
        ratings,
        eligible_subject,
    }))
}

fn rating_response(
    rating: RequestRating,
    participants: &BTreeMap<String, RequestRatingParticipantResponse>,
) -> Result<RequestRatingResponse, ApiError> {
    Ok(RequestRatingResponse {
        rater: participant_response(&rating.rater_user_id, participants)?,
        subject: participant_response(&rating.subject_user_id, participants)?,
        id: rating.id,
        request_id: rating.request_id,
        score: rating.score,
        reason: rating.reason,
        created_at_unix: rating.created_at_unix,
    })
}

async fn rating_participants(
    state: &AppState,
    users: &BTreeMap<String, UserAccount>,
) -> Result<BTreeMap<String, RequestRatingParticipantResponse>, ApiError> {
    let mut participants = BTreeMap::new();
    for user in users.values() {
        let reputation = state
            .metadata
            .requests()
            .request_reputation(&user.id)
            .await?;
        participants.insert(
            user.id.clone(),
            RequestRatingParticipantResponse {
                id: user.id.clone(),
                handle: user.handle.clone(),
                rating_score_sum: reputation.score_sum,
                rating_count: reputation.rating_count,
            },
        );
    }
    Ok(participants)
}

fn participant_response(
    user_id: &str,
    participants: &BTreeMap<String, RequestRatingParticipantResponse>,
) -> Result<RequestRatingParticipantResponse, ApiError> {
    participants
        .get(user_id)
        .cloned()
        .ok_or_else(|| ApiError::internal_message("request rating participant is missing"))
}
