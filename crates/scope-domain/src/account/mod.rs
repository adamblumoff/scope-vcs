pub mod cli_auth;
pub mod handles;

use crate::store::UserAccount;
use serde::{Deserialize, Serialize};

/// The authenticated account facts exposed to a signed-in session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalIdentity {
    pub provider: String,
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SessionIdentity {
    pub user_id: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

impl From<&UserAccount> for SessionIdentity {
    fn from(user: &UserAccount) -> Self {
        Self {
            user_id: user.id.clone(),
            email: (!user.email.is_empty()).then(|| user.email.clone()),
            email_verified: user.email_verified,
        }
    }
}
