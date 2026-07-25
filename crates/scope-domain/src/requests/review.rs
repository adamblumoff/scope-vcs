use super::{REQUEST_ASSESSMENT_BODY_MAX_BYTES, validate_body_size};
use crate::error::DomainError;
use serde::{Deserialize, Serialize};

pub const REQUEST_MAX_STAKE_CREDITS: u32 = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestAssessmentOutcome {
    Accepted,
    Neutral,
    Rejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestReviewExitReason {
    AuthorReturned,
    ChangesRequested,
    RevisionPushed,
    ContentEdited,
}

pub fn validate_assessment_body(
    outcome: RequestAssessmentOutcome,
    body_markdown: Option<&str>,
) -> Result<(), DomainError> {
    if let Some(body) = body_markdown {
        validate_body_size("assessment body", body, REQUEST_ASSESSMENT_BODY_MAX_BYTES)?;
    }
    if outcome == RequestAssessmentOutcome::Rejected
        && body_markdown.is_none_or(|body| body.trim().is_empty())
    {
        Err(DomainError::invalid_input(
            "rejected assessment requires a written reason",
        ))
    } else {
        Ok(())
    }
}
