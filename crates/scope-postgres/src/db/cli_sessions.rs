use super::{
    auth::{i64_to_u64, load_user_by_id, u64_to_i64},
    cli_auth_results::{CliSessionSummary, NewCliSession},
    entities,
};
use crate::error::PostgresError;
use scope_domain::account::SessionIdentity;
use sea_orm::{ActiveModelTrait, IntoActiveModel};

pub async fn insert_cli_session_in_tx<C>(
    conn: &C,
    user_id: &str,
    session: NewCliSession,
) -> Result<SessionIdentity, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    entities::cli_session::Model {
        id: session.id,
        token_hash: session.token_hash,
        user_id: user_id.to_string(),
        label: session.label,
        created_at_unix: u64_to_i64(session.created_at_unix)?,
        last_used_at_unix: None,
        expires_at_unix: u64_to_i64(session.expires_at_unix)?,
        revoked_at_unix: None,
    }
    .into_active_model()
    .insert(conn)
    .await
    .map_err(PostgresError::internal)?;
    let user = load_user_by_id(conn, user_id).await?;
    Ok(SessionIdentity::from(&user))
}

pub fn cli_session_summary_from_model(
    session: entities::cli_session::Model,
) -> Result<CliSessionSummary, PostgresError> {
    Ok(CliSessionSummary {
        id: session.id,
        label: session.label,
        created_at_unix: i64_to_u64(session.created_at_unix)?,
        last_used_at_unix: session.last_used_at_unix.map(i64_to_u64).transpose()?,
        expires_at_unix: i64_to_u64(session.expires_at_unix)?,
    })
}
