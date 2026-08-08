use crate::{
    auth::{scope::require_scope_user, tokens::generate_runner_token},
    error::ApiError,
    persistence::unix_now,
    persistence_ids::generate_prefixed_id,
    repo_access::find_repo,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use scope_api_contract::{
    AttachRunnerRepositoryRequest, RegisterRunnerRequest, RegisterRunnerResponse,
    RunnerGrantResponse, RunnerResponse, UpgradeRunnerRegistrationRequest,
    UpgradeRunnerRegistrationResponse,
};
use scope_domain::{
    runs::runner::{Runner, RunnerGrant, RunnerName},
    store::RepositoryActor,
};
use scope_postgres::db::UpgradeRunnerRegistrationCommand;

pub(crate) async fn register_runner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RegisterRunnerRequest>,
) -> Result<Json<RegisterRunnerResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = require_repo_member(&state, &user.id, &input.owner, &input.repo).await?;
    let (secret, secret_hash) = generate_runner_token()?;
    let now = unix_now()?;
    let runner = Runner::new(
        generate_prefixed_id("runner_")?,
        &user.id,
        secret_hash,
        input.version,
        input.protocol_version,
        input.capabilities,
        input.max_concurrent_jobs,
        now,
    )?;
    let grant = RunnerGrant::new(
        &repo.record.id,
        &runner.id,
        RunnerName::parse(input.name)?,
        &user.id,
        now,
    )?;
    let (runner, grant) = state
        .metadata
        .runs()
        .register_runner_with_grant(runner, grant)
        .await?;
    Ok(Json(RegisterRunnerResponse {
        runner: runner_response(runner, vec![grant]),
        secret,
    }))
}

pub(crate) async fn get_runner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(runner_id): Path<String>,
) -> Result<Json<RunnerResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let runner = require_owned_runner(&state, &runner_id, &user.id).await?;
    let grants = state.metadata.runs().runner_grants(&runner.id).await?;
    Ok(Json(runner_response(runner, grants)))
}

pub(crate) async fn upgrade_runner_registration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(runner_id): Path<String>,
    Json(input): Json<UpgradeRunnerRegistrationRequest>,
) -> Result<Json<UpgradeRunnerRegistrationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (secret, secret_hash) = generate_runner_token()?;
    let runner = state
        .metadata
        .runs()
        .upgrade_runner_registration(
            &runner_id,
            &user.id,
            UpgradeRunnerRegistrationCommand {
                secret_hash,
                version: input.version,
                protocol_version: input.protocol_version,
                capabilities: input.capabilities,
                max_concurrent_jobs: input.max_concurrent_jobs,
            },
        )
        .await?;
    let grants = state.metadata.runs().runner_grants(&runner.id).await?;
    Ok(Json(UpgradeRunnerRegistrationResponse {
        runner: runner_response(runner, grants),
        secret,
    }))
}

pub(crate) async fn delete_runner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(runner_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    require_owned_runner(&state, &runner_id, &user.id).await?;
    state
        .metadata
        .runs()
        .delete_unused_runner(&runner_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn attach_runner_repository(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((runner_id, owner, repo_name)): Path<(String, String, String)>,
    Json(input): Json<AttachRunnerRepositoryRequest>,
) -> Result<Json<RunnerResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let runner = require_owned_runner(&state, &runner_id, &user.id).await?;
    let repo = require_repo_member(&state, &user.id, &owner, &repo_name).await?;
    state
        .metadata
        .runs()
        .grant_runner(RunnerGrant::new(
            &repo.record.id,
            &runner.id,
            RunnerName::parse(input.name)?,
            &user.id,
            unix_now()?,
        )?)
        .await?;
    let grants = state.metadata.runs().runner_grants(&runner.id).await?;
    Ok(Json(runner_response(runner, grants)))
}

pub(crate) async fn detach_runner_repository(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((runner_id, owner, repo_name)): Path<(String, String, String)>,
) -> Result<Json<RunnerResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let runner = require_owned_runner(&state, &runner_id, &user.id).await?;
    let repo = require_repo_member(&state, &user.id, &owner, &repo_name).await?;
    state
        .metadata
        .runs()
        .revoke_runner_grant(&repo.record.id, &runner.id, unix_now()?)
        .await?;
    let grants = state.metadata.runs().runner_grants(&runner.id).await?;
    Ok(Json(runner_response(runner, grants)))
}

async fn require_owned_runner(
    state: &AppState,
    runner_id: &str,
    user_id: &str,
) -> Result<Runner, ApiError> {
    let runner = state
        .metadata
        .runs()
        .runner(runner_id)
        .await?
        .ok_or_else(|| ApiError::not_found("runner not found"))?;
    if runner.owner_user_id != user_id {
        return Err(ApiError::not_found("runner not found"));
    }
    Ok(runner)
}

async fn require_repo_member(
    state: &AppState,
    user_id: &str,
    owner: &str,
    name: &str,
) -> Result<scope_domain::store::StoredRepository, ApiError> {
    let repo = find_repo(state, owner, name).await?;
    if repo.access_for_user_id(user_id).actor == RepositoryActor::Public {
        return Err(ApiError::forbidden("repo membership required"));
    }
    Ok(repo)
}

pub(crate) fn runner_response(runner: Runner, grants: Vec<RunnerGrant>) -> RunnerResponse {
    RunnerResponse {
        id: runner.id,
        version: runner.version,
        protocol_version: runner.protocol_version,
        max_concurrent_jobs: runner.max_concurrent_jobs,
        enabled: runner.enabled,
        created_at_unix: runner.created_at_unix,
        last_seen_at_unix: runner.last_seen_at_unix,
        grants: grants
            .into_iter()
            .map(|grant| {
                let active = grant.is_active();
                RunnerGrantResponse {
                    repository_id: grant.repository_id,
                    name: grant.name.as_str().to_string(),
                    active,
                }
            })
            .collect(),
    }
}
