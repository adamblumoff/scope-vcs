#!/usr/bin/env bash
set -euo pipefail

cache_upload_root="${1:?usage: deploy-staging-railway.sh <cache-root> <worker-root> <api-root> <web-root>}"
worker_upload_root="${2:?usage: deploy-staging-railway.sh <cache-root> <worker-root> <api-root> <web-root>}"
api_upload_root="${3:?usage: deploy-staging-railway.sh <cache-root> <worker-root> <api-root> <web-root>}"
web_upload_root="${4:?usage: deploy-staging-railway.sh <cache-root> <worker-root> <api-root> <web-root>}"
manifest_path="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
maintenance_binary="${SCOPE_MAINTENANCE_BINARY:-./target/release/scope-maintenance}"
seed_binary="${SCOPE_SMOKE_SEED_BINARY:-./target/release/scope-smoke-seed}"
evidence_path="${SCOPE_STAGING_EVIDENCE_PATH:-staging-deployments.json}"

if [[ -z "${RAILWAY_TOKEN:-}" || -n "${RAILWAY_API_TOKEN:-}" ]]; then
  echo "Staging deployment requires only a staging-scoped RAILWAY_TOKEN." >&2
  exit 1
fi
for binary in "$maintenance_binary" "$seed_binary"; do
  if [[ ! -x "$binary" ]]; then
    echo "Required staging command is not executable: $binary" >&2
    exit 1
  fi
done

manifest_json="$(jq -c . "$manifest_path")"
project_id="$(jq -er '.railway.projectId' "$manifest_path")"
production_environment_id="$(jq -er '.railway.environmentId' "$manifest_path")"
staging_environment_id="$(jq -er '.railway.staging.environmentId' "$manifest_path")"
staging_environment_name="$(jq -er '.railway.staging.environmentName' "$manifest_path")"
staging_cache_url="https://$(jq -er '.railway.staging.cacheDomain' "$manifest_path")"
staging_region="$(jq -er '.railway.regionId' "$manifest_path")"
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

evidence_lines="$(mktemp)"
api_closed=0
worker_closed=0
cutover_committed=0

scale_service() {
  local service="$1"
  local replicas="$2"
  railway service scale "${railway_scope[@]}" --service "$service" \
    "$staging_region=$replicas" --json >/dev/null
}

wait_for_replica_state() {
  local service="$1"
  local expected_running="$2"
  local deadline=$((SECONDS + 300))
  local state
  local status configured running crashed

  while [[ "$SECONDS" -lt "$deadline" ]]; do
    state="$(
      railway service list "${railway_scope[@]}" --json |
        jq -er --arg service "$service" '
          .[] | select(.id == $service) |
          [.status, (.replicas.configured // 0), (.replicas.running // 0), (.replicas.crashed // 0)] |
          @tsv
        '
    )"
    IFS=$'\t' read -r status configured running crashed <<< "$state"
    if [[ "$expected_running" == "0" && "$configured" == "0" && "$running" == "0" ]]; then
      return 0
    fi
    if [[ "$expected_running" == "1" && "$status" == "SUCCESS" \
      && "$configured" -ge 1 && "$running" -ge 1 && "$crashed" == "0" ]]; then
      return 0
    fi
    sleep 5
  done

  echo "Timed out waiting for staging service $service to reach $expected_running running replicas." >&2
  return 1
}

cleanup() {
  local exit_status=$?
  trap - EXIT
  if [[ "$exit_status" -ne 0 && "$cutover_committed" == "0" ]]; then
    [[ "$worker_closed" == "0" ]] || scale_service "$worker_service" 1 || true
    [[ "$api_closed" == "0" ]] || scale_service "$api_service" 1 || true
  elif [[ "$exit_status" -ne 0 && "$cutover_committed" == "1" ]]; then
    scale_service "$api_service" 0 || true
    scale_service "$worker_service" 0 || true
    echo "Staging migration committed; writers remain closed until the proof is rerun." >&2
  fi
  rm -f "$evidence_lines"
  exit "$exit_status"
}
trap cleanup EXIT

scale_service "$api_service" 0
api_closed=1
scale_service "$worker_service" 0
worker_closed=1
wait_for_replica_state "$api_service" 0
wait_for_replica_state "$worker_service" 0

# The remote shell expands the Railway-provided database URL.
# shellcheck disable=SC2016
railway run "${railway_scope[@]}" --service "$database_service" --no-local -- \
  sh -c 'DATABASE_URL="$DATABASE_PUBLIC_URL" exec "$@"' \
  scope-maintenance "$maintenance_binary" apply
cutover_committed=1

deploy_service() {
  local service="$1"
  local upload_root="$2"
  local deployment_json

  bash .github/scripts/deploy-railway.sh "$service" "$upload_root"
  deployment_json="$(
    railway deployment list "${railway_scope[@]}" --service "$service" --limit 1 --json |
      jq -ec --arg service "$service" '
        first | select(.status == "SUCCESS") |
        {service: $service, deploymentId: .id, status: .status}
      '
  )"
  printf '%s\n' "$deployment_json" >> "$evidence_lines"
}

deploy_writer() {
  local service="$1"
  local upload_root="$2"
  local deployment_json

  bash .github/scripts/deploy-railway.sh "$service" "$upload_root" stopped
  scale_service "$service" 1
  wait_for_replica_state "$service" 1
  deployment_json="$(
    railway deployment list "${railway_scope[@]}" --service "$service" --limit 1 --json |
      jq -ec --arg service "$service" '
        first | select(.status == "SUCCESS") |
        {service: $service, deploymentId: .id, status: .status}
      '
  )"
  printf '%s\n' "$deployment_json" >> "$evidence_lines"
}

deploy_service "$cache_service" "$cache_upload_root"
deploy_writer "$worker_service" "$worker_upload_root"
worker_closed=0
deploy_writer "$api_service" "$api_upload_root"
api_closed=0

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

deploy_service "$web_service" "$web_upload_root"

jq -s \
  --arg commit "${GITHUB_SHA:-unknown}" \
  --arg environmentId "$staging_environment_id" \
  '{commit: $commit, environmentId: $environmentId, deployments: .}' \
  "$evidence_lines" > "$evidence_path"
