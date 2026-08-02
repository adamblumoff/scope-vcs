use super::{Request, validate_body_size, validate_required_body, validate_required_id};
use crate::error::DomainError;

pub const REQUEST_RATING_REASON_MAX_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestRating {
    pub id: String,
    pub request_id: String,
    pub rater_user_id: String,
    pub subject_user_id: String,
    pub score: u8,
    pub reason: String,
    pub created_at_unix: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequestReputation {
    pub score_sum: u64,
    pub rating_count: u64,
}

impl RequestReputation {
    pub fn from_totals(score_sum: u64, rating_count: u64) -> Result<Self, DomainError> {
        let maximum_score = rating_count.checked_mul(5).ok_or_else(|| {
            DomainError::invariant_violation("request reputation rating count overflow")
        })?;
        if (rating_count == 0 && score_sum != 0)
            || (rating_count > 0 && !(rating_count..=maximum_score).contains(&score_sum))
        {
            return Err(DomainError::invariant_violation(
                "request reputation totals are inconsistent",
            ));
        }
        Ok(Self {
            score_sum,
            rating_count,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateRequestRatingInput {
    pub id: String,
    pub request_id: String,
    pub actor_user_id: String,
    pub score: u8,
    pub reason: String,
    pub now_unix: u64,
}

pub fn eligible_rating_subject_user_id<'a>(
    request: &'a Request,
    actor_user_id: &str,
    existing_ratings: &[RequestRating],
) -> Option<&'a str> {
    if existing_ratings
        .iter()
        .any(|rating| rating.rater_user_id == actor_user_id)
    {
        return None;
    }
    let terminal_actor = request
        .merged_by_user_id
        .as_deref()
        .or(request.closed_by_user_id.as_deref())?;
    if request.author_user_id == terminal_actor {
        return None;
    }
    if actor_user_id == request.author_user_id {
        Some(terminal_actor)
    } else if actor_user_id == terminal_actor {
        Some(&request.author_user_id)
    } else {
        None
    }
}

pub fn create_request_rating(
    request: &Request,
    existing_ratings: &[RequestRating],
    input: CreateRequestRatingInput,
) -> Result<RequestRating, DomainError> {
    validate_required_id("rating id", &input.id)?;
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("rating actor", &input.actor_user_id)?;
    if input.request_id != request.id {
        return Err(DomainError::conflict("rating request does not match"));
    }
    if !(1..=5).contains(&input.score) {
        return Err(DomainError::invalid_input(
            "request rating score must be between 1 and 5",
        ));
    }
    validate_required_body("request rating reason", &input.reason)?;
    let reason = input.reason.trim().to_string();
    validate_body_size(
        "request rating reason",
        &reason,
        REQUEST_RATING_REASON_MAX_BYTES,
    )?;
    if existing_ratings.len() >= 2 {
        return Err(DomainError::conflict(
            "request already has both participant ratings",
        ));
    }
    let subject_user_id =
        eligible_rating_subject_user_id(request, &input.actor_user_id, existing_ratings)
            .ok_or_else(|| DomainError::forbidden("actor is not eligible to rate this request"))?
            .to_string();

    Ok(RequestRating {
        id: input.id,
        request_id: input.request_id,
        rater_user_id: input.actor_user_id,
        subject_user_id,
        score: input.score,
        reason,
        created_at_unix: input.now_unix,
    })
}
