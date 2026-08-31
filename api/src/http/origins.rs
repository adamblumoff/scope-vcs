use crate::{
    config::{LOCAL_APP_ORIGIN, SCOPE_APP_ORIGIN_ENV, non_empty_env},
    error::ApiError,
    state::AppState,
};

pub(crate) fn public_git_origin(state: &AppState) -> &str {
    &state.git_public_url
}

pub(crate) fn public_app_origin(action: &str) -> Result<String, ApiError> {
    public_origin(SCOPE_APP_ORIGIN_ENV, LOCAL_APP_ORIGIN, action)
}

fn public_origin(env_name: &str, debug_fallback: &str, action: &str) -> Result<String, ApiError> {
    non_empty_env(env_name)
        .or_else(|| cfg!(debug_assertions).then(|| debug_fallback.to_string()))
        .map(|value| value.trim_end_matches('/').to_string())
        .ok_or_else(|| {
            ApiError::infrastructure_unavailable(format!("{env_name} is required to {action}"))
        })
}
