use super::{
    auth::{i64_to_u64, load_user_by_id, u64_to_i64},
    entities,
};
use crate::auth::device::SessionIdentity;
use crate::{
    auth::{
        device::{CliSessionSummary, CliSessionToken, random_prefixed_token},
        tokens::token_hash,
    },
    config::{CLI_SESSION_ID_PREFIX, CLI_SESSION_TOKEN_PREFIX, CLI_SESSION_TTL_SECS},
    error::ApiError,
};
use sea_orm::{ActiveModelTrait, IntoActiveModel};

pub async fn create_cli_session_token_in_tx<C>(
    conn: &C,
    user_id: &str,
    now: u64,
) -> Result<CliSessionToken, ApiError>
where
    C: sea_orm::ConnectionTrait,
{
    let session_id = random_prefixed_token(CLI_SESSION_ID_PREFIX)?;
    let session_token = random_prefixed_token(CLI_SESSION_TOKEN_PREFIX)?;
    let expires_at_unix = now + CLI_SESSION_TTL_SECS;
    entities::cli_session::Model {
        id: session_id,
        token_hash: token_hash(&session_token),
        user_id: user_id.to_string(),
        label: "Scope CLI".to_string(),
        created_at_unix: u64_to_i64(now)?,
        last_used_at_unix: None,
        expires_at_unix: u64_to_i64(expires_at_unix)?,
        revoked_at_unix: None,
    }
    .into_active_model()
    .insert(conn)
    .await
    .map_err(ApiError::internal)?;
    let user = load_user_by_id(conn, user_id).await?;
    Ok(CliSessionToken {
        session_token,
        expires_at_unix,
        identity: SessionIdentity::from(&user),
    })
}

pub fn cli_session_summary_from_model(
    session: entities::cli_session::Model,
) -> Result<CliSessionSummary, ApiError> {
    Ok(CliSessionSummary {
        id: session.id,
        label: session.label,
        created_at_unix: i64_to_u64(session.created_at_unix)?,
        last_used_at_unix: session.last_used_at_unix.map(i64_to_u64).transpose()?,
        expires_at_unix: i64_to_u64(session.expires_at_unix)?,
    })
}
