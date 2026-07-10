#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    BadRequest,
    Conflict,
    Forbidden,
    Internal,
    NotFound,
    PayloadTooLarge,
    ServiceUnavailable,
    TooManyRequests,
    Unauthorized,
}

#[derive(Debug)]
pub struct ApiError {
    pub kind: ErrorKind,
    pub message: String,
}
macro_rules! message_errors {
    ($($name:ident => $kind:ident),+ $(,)?) => {$(
        pub fn $name(message: impl Into<String>) -> Self {
            Self::new(ErrorKind::$kind, message)
        }
    )+};
}
impl ApiError {
    pub fn bad_request(error: impl std::fmt::Display) -> Self {
        Self::new(ErrorKind::BadRequest, error.to_string())
    }
    pub fn internal(error: impl std::error::Error) -> Self {
        Self::new(ErrorKind::Internal, error.to_string())
    }
    message_errors! {
        forbidden => Forbidden,
        conflict => Conflict,
        payload_too_large => PayloadTooLarge,
        too_many_requests => TooManyRequests,
        unauthorized => Unauthorized,
        not_found => NotFound,
        internal_message => Internal,
        service_unavailable => ServiceUnavailable,
    }
    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}
