use crate::policy::PolicyError;

pub type ReviewedUpdateResult<T> = Result<T, ReviewedUpdateError>;

#[derive(Debug)]
pub enum ReviewedUpdateError {
    BadRequest(&'static str),
    Conflict(&'static str),
    InvalidPolicy(PolicyError),
}
