#!/usr/bin/env bash
set -euo pipefail

action="${1:?usage: deploy-staging-railway.sh <prepare|finish> <upload-roots...>}"
shift
manifest_path="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
maintenance_binary="${SCOPE_MAINTENANCE_BINARY:-./target/release/scope-maintenance}"
seed_binary="${SCOPE_SMOKE_SEED_BINARY:-./target/release/scope-smoke-seed}"
evidence_path="${SCOPE_STAGING_EVIDENCE_PATH:-staging-deployments.json}"

if [[ -z "${RAILWAY_TOKEN:-}" || -n "${RAILWAY_API_TOKEN:-}" ]]; then
  echo "Staging deployment requires only a staging-scoped RAILWAY_TOKEN." >&2
  exit 1
fi

manifest_json="$(jq -c . "$manifest_path")"
project_id="$(jq -er '.railway.projectId' "$manifest_path")"
production_environment_id="$(jq -er '.railway.environmentId' "$manifest_path")"
staging_environment_id="$(jq -er '.railway.staging.environmentId' "$manifest_path")"
staging_environment_name="$(jq -er '.railway.staging.environmentName' "$manifest_path")"
staging_cache_url="https://$(jq -er '.railway.staging.cacheDomain' "$manifest_path")"
database_service="$(jq -er '.railway.databaseServiceId' "$manifest_path")"
cache_service="$(jq -er '.services.cache.id' "$manifest_path")"
worker_service="$(jq -er '.services.worker.id' "$manifest_path")"
api_service="$(jq -er '.services.api.id' "$manifest_path")"
web_service="$(jq -er '.services.web.id' "$manifest_path")"

if [[ "$staging_environment_id" == "$production_environment_id" ]]; then
  echo "Staging environment matches production." >&2
  exit 1
fi

railway_scope=(
  --project "$project_id"
  --environment "$staging_environment_id"
)
status_json="$(railway status "${railway_scope[@]}" --json)"
services_json="$(railway service list "${railway_scope[@]}" --json)"
SCOPE_DEPLOYMENT_MANIFEST_JSON="$manifest_json" \
  SCOPE_RAILWAY_STATUS_JSON="$status_json" \
  SCOPE_RAILWAY_SERVICES_JSON="$services_json" \
  node .github/scripts/verify-staging-target.mjs >/dev/null

api_variables="$(railway variable list "${railway_scope[@]}" --service "$api_service" --json)"
if ! jq -e --arg expected "$staging_cache_url" '.SCOPE_CACHE_URL == $expected' \
  <<< "$api_variables" >/dev/null; then
  echo "Staging API SCOPE_CACHE_URL does not match the reviewed staging cache domain." >&2
  exit 1
fi

export RAILWAY_PROJECT_ID="$project_id"
export SCOPE_RAILWAY_ENVIRONMENT_ID="$staging_environment_id"
export RAILWAY_DEPLOY_MESSAGE="Staging ${GITHUB_SHA:-candidate}"

assert_writer_state() {
  local expected_running="$1"
  local current_services
  current_services="$(railway service list "${railway_scope[@]}" --json)"
  SERVICES_JSON="$current_services" \
    API_SERVICE="$api_service" \
    WORKER_SERVICE="$worker_service" \
    EXPECTED_RUNNING="$expected_running" \
    node -e '
const services = JSON.parse(process.env.SERVICES_JSON || "[]");
const expected = Number(process.env.EXPECTED_RUNNING);
for (const id of [process.env.API_SERVICE, process.env.WORKER_SERVICE]) {
  const service = services.find((candidate) => candidate.id === id);
  if (!service) process.exit(1);
  const replicas = service.replicas || {};
  if (expected === 0 && (replicas.running || 0) === 0 && (replicas.crashed || 0) === 0) continue;
  if (expected === 1 && service.status === "SUCCESS" && (replicas.configured || 0) >= 1 &&
      (replicas.running || 0) >= 1 && (replicas.crashed || 0) === 0) continue;
  process.exit(1);
}
'
}

record_deployment() {
  local service="$1"
  railway deployment list "${railway_scope[@]}" --service "$service" --limit 1 --json |
    jq -ec --arg service "$service" '
      first | select(.status == "SUCCESS") |
      {service: $service, deploymentId: .id, status: .status}
    '
}

case "$action" in
  prepare)
    if [[ "$#" -ne 3 || ! -x "$maintenance_binary" ]]; then
      echo "usage: deploy-staging-railway.sh prepare <cache-root> <worker-root> <api-root>" >&2
      exit 2
    fi
    assert_writer_state 0
    # The remote shell expands the Railway-provided database URL.
    # shellcheck disable=SC2016
    railway run "${railway_scope[@]}" --service "$database_service" --no-local -- \
      sh -c 'DATABASE_URL="$DATABASE_PUBLIC_URL" exec "$@"' \
      scope-maintenance "$maintenance_binary" apply
    bash .github/scripts/deploy-railway.sh "$cache_service" "$1"
    bash .github/scripts/deploy-railway.sh "$worker_service" "$2"
    bash .github/scripts/deploy-railway.sh "$api_service" "$3"
    ;;
  finish)
    if [[ "$#" -ne 1 || ! -x "$seed_binary" ]]; then
      echo "usage: deploy-staging-railway.sh finish <web-root>" >&2
      exit 2
    fi
    assert_writer_state 1
    database_variables="$(railway variable list "${railway_scope[@]}" --service "$database_service" --json)"
    SCOPE_STAGING_DATABASE_PUBLIC_URL="$(
      jq -er '.DATABASE_PUBLIC_URL | strings | select(length > 0)' <<< "$database_variables"
    )"
    export SCOPE_STAGING_DATABASE_PUBLIC_URL
    export SCOPE_ALLOW_STAGING_SMOKE_SEED=1
    export SCOPE_SMOKE_SEED_PROJECT_ID="$project_id"
    export SCOPE_SMOKE_SEED_ENVIRONMENT_ID="$staging_environment_id"
    export SCOPE_SMOKE_SEED_ENVIRONMENT_NAME="$staging_environment_name"
    export SCOPE_PRODUCTION_ENVIRONMENT_ID="$production_environment_id"
    export SCOPE_SMOKE_SEED_USER_EMAIL="smoke@example.test"
    export SCOPE_SMOKE_SEED_USER_HANDLE="dev"
    # The remote shell expands the locally supplied staging URL.
    # shellcheck disable=SC2016
    railway run "${railway_scope[@]}" --service "$api_service" --no-local -- \
      sh -c 'DATABASE_URL="$SCOPE_STAGING_DATABASE_PUBLIC_URL" exec "$@"' \
      scope-smoke-seed "$seed_binary"
    unset SCOPE_STAGING_DATABASE_PUBLIC_URL

    bash .github/scripts/deploy-railway.sh "$web_service" "$1"
    evidence_lines="$(mktemp)"
    trap 'rm -f "$evidence_lines"' EXIT
    for service in "$cache_service" "$worker_service" "$api_service" "$web_service"; do
      record_deployment "$service" >> "$evidence_lines"
    done
    jq -s \
      --arg commit "${GITHUB_SHA:-unknown}" \
      --arg environmentId "$staging_environment_id" \
      '{commit: $commit, environmentId: $environmentId, deployments: .}' \
      "$evidence_lines" > "$evidence_path"
    ;;
  *)
    echo "usage: deploy-staging-railway.sh <prepare|finish> <upload-roots...>" >&2
    exit 2
    ;;
esac
