use crate::error::ApiError;
use serde::{Deserialize, Serialize};

pub const REQUEST_MAX_STAKE_CREDITS: u32 = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestAssessmentOutcome {
    Accepted,
    Neutral,
    Rejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestReviewExitReason {
    AuthorReturned,
    ChangesRequested,
    RevisionPushed,
}

pub fn validate_assessment_body(
    outcome: RequestAssessmentOutcome,
    body_markdown: Option<&str>,
) -> Result<(), ApiError> {
    if outcome == RequestAssessmentOutcome::Rejected
        && body_markdown.is_none_or(|body| body.trim().is_empty())
    {
        Err(ApiError::bad_request(
            "rejected assessment requires a written reason",
        ))
    } else {
        Ok(())
    }
}
