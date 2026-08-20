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

export RAILWAY_PROJECT_ID="$project_id"
export SCOPE_RAILWAY_ENVIRONMENT_ID="$staging_environment_id"
export RAILWAY_DEPLOY_MESSAGE="Staging ${GITHUB_SHA:-candidate}"

# The remote shell expands the Railway-provided database URL.
# shellcheck disable=SC2016
railway run "${railway_scope[@]}" --service "$database_service" --no-local -- \
  sh -c 'DATABASE_URL="$DATABASE_PUBLIC_URL" exec "$@"' \
  scope-maintenance "$maintenance_binary" apply

evidence_lines="$(mktemp)"
trap 'rm -f "$evidence_lines"' EXIT

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

deploy_service "$cache_service" "$cache_upload_root"
deploy_service "$worker_service" "$worker_upload_root"
deploy_service "$api_service" "$api_upload_root"

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
