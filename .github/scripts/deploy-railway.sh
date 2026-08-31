#!/usr/bin/env bash
set -euo pipefail

service_name="${1:?usage: deploy-railway.sh <service-name> <upload-root>}"
upload_root="${2:?usage: deploy-railway.sh <service-name> <upload-root>}"

if [[ -z "${RAILWAY_API_TOKEN:-}" && -z "${RAILWAY_TOKEN:-}" ]]; then
  echo "Set RAILWAY_API_TOKEN or RAILWAY_TOKEN before deploying ${service_name}."
  exit 1
fi

if [[ -n "${RAILWAY_API_TOKEN:-}" && -n "${RAILWAY_TOKEN:-}" ]]; then
  echo "Set only one Railway token type before deploying ${service_name}."
  exit 1
fi

if [ -z "${RAILWAY_PROJECT_ID:-}" ]; then
  echo "Set RAILWAY_PROJECT_ID before deploying ${service_name}."
  exit 1
fi

railway_environment="${SCOPE_RAILWAY_ENVIRONMENT_ID:-production}"
deployment_component="${SCOPE_DEPLOYMENT_COMPONENT:-}"
deployment_source_sha="${SCOPE_DEPLOYMENT_SOURCE_SHA:-${GITHUB_SHA:-}}"
deployment_evidence_path="${SCOPE_DEPLOYMENT_EVIDENCE_PATH:-}"
verified_successful_sha="${SCOPE_VERIFIED_SUCCESSFUL_SHA:-}"
defer_service_health="${SCOPE_DEFER_SERVICE_HEALTH:-0}"
deployment_was_skipped=0

if [[ "$defer_service_health" != "0" && "$defer_service_health" != "1" ]]; then
  echo "SCOPE_DEFER_SERVICE_HEALTH must be 0 or 1." >&2
  exit 2
fi

deploy_message_from_event() {
  local raw_message="${RAILWAY_DEPLOY_MESSAGE:-}"
  local first_line
  local pr_title

  first_line="$(printf '%s\n' "$raw_message" | sed -n '1p')"
  pr_title="$(printf '%s\n' "$raw_message" | awk 'NR > 1 && NF { print; exit }')"

  if [[ "$first_line" =~ ^Merge\ pull\ request\ #[0-9]+ ]] && [ -n "$pr_title" ]; then
    printf '%s\n' "$pr_title"
  elif [ -n "$first_line" ]; then
    printf '%s\n' "$first_line"
  else
    printf '%s\n' "${GITHUB_WORKFLOW:-Railway deploy}"
  fi
}

ensure_service_exists() {
  local service_name="$1"
  local services_json

  services_json="$(
    railway service list \
      --project "$RAILWAY_PROJECT_ID" \
      --environment "$railway_environment" \
      --json
  )"

  if ! SERVICES_JSON="$services_json" SERVICE_NAME="$service_name" node -e 'const services = JSON.parse(process.env.SERVICES_JSON || "[]"); const name = process.env.SERVICE_NAME || ""; process.exit(services.some((service) => service.name === name || service.id === name) ? 0 : 1);'; then
    echo "Railway service '${service_name}' was not found in environment '${railway_environment}'."
    echo "Create the service in Railway, configure its variables, then rerun this workflow."
    return 1
  fi
}

service_is_healthy() {
  local service_name="$1"
  local expected_deployment_id="${2:-}"
  local services_json
  services_json="$(
    railway service list \
      --project "$RAILWAY_PROJECT_ID" \
      --environment "$railway_environment" \
      --json
  )"
  SCOPE_RAILWAY_SERVICES_JSON="$services_json" \
    SCOPE_RAILWAY_SERVICE_ID="$service_name" \
    SCOPE_EXPECTED_RAILWAY_DEPLOYMENT_ID="$expected_deployment_id" \
    node .github/scripts/railway-service-health.mjs >/dev/null
}

wait_for_service_health() {
  local service_name="$1"
  local expected_deployment_id="${2:-}"
  local timeout="${SCOPE_SERVICE_HEALTH_TIMEOUT_SECONDS:-600}"
  local interval="${SCOPE_SERVICE_HEALTH_POLL_SECONDS:-10}"
  local deadline=$((SECONDS + timeout))
  while true; do
    if service_is_healthy "$service_name" "$expected_deployment_id" 2>/dev/null; then
      return 0
    fi
    (( SECONDS < deadline )) || break
    sleep "$interval"
  done
  service_is_healthy "$service_name" "$expected_deployment_id" || true
  echo "Timed out waiting for $service_name to reach its exact healthy deployment." >&2
  return 1
}

print_deployment_logs() {
  local service_name="$1"
  local deployment_id="$2"

  echo "::group::Railway build logs for ${service_name}/${deployment_id}"
  railway logs "$deployment_id" \
    --project "$RAILWAY_PROJECT_ID" \
    --service "$service_name" \
    --environment "$railway_environment" \
    --build \
    --lines 200 || true
  echo "::endgroup::"

  echo "::group::Railway deploy logs for ${service_name}/${deployment_id}"
  railway logs "$deployment_id" \
    --project "$RAILWAY_PROJECT_ID" \
    --service "$service_name" \
    --environment "$railway_environment" \
    --deployment \
    --lines 200 || true
  echo "::endgroup::"
}

upload_contains_source_revision() {
  local marker
  [[ -n "$deployment_source_sha" ]] || return 1

  while IFS= read -r marker; do
    if [[ "$(tr -d '[:space:]' < "$marker")" == "$deployment_source_sha" ]]; then
      return 0
    fi
  done < <(find "$upload_root" -type f -name .scope-deployment-sha -print)
  return 1
}

record_deployment_evidence() {
  local deployment_id="$1"
  [[ -n "$deployment_evidence_path" ]] || return 0
  if [[ -z "$deployment_component" || -z "$deployment_source_sha" ]]; then
    echo "SCOPE_DEPLOYMENT_COMPONENT and SCOPE_DEPLOYMENT_SOURCE_SHA are required when recording evidence." >&2
    return 1
  fi
  if ! upload_contains_source_revision; then
    echo "Railway upload for ${deployment_component} does not contain source revision ${deployment_source_sha}." >&2
    return 1
  fi

  # The JavaScript template literal is evaluated by Node.
  # shellcheck disable=SC2016
  EVIDENCE_PATH="$deployment_evidence_path" \
    COMPONENT="$deployment_component" \
    SOURCE_SHA="$deployment_source_sha" \
    DEPLOYMENT_ID="$deployment_id" \
    node -e '
const { appendFileSync } = require("node:fs");
appendFileSync(process.env.EVIDENCE_PATH, `${JSON.stringify({
  component: process.env.COMPONENT,
  sourceSha: process.env.SOURCE_SHA,
  provider: "railway",
  evidenceId: process.env.DEPLOYMENT_ID,
})}\n`);
'
}

wait_for_deployment() {
  local service_name="$1"
  local deployment_id="$2"
  local deadline=$((SECONDS + 900))
  local deployment_json
  local deployment_line
  local deployment_status
  local skipped_reason

  while true; do
    if deployment_json="$(
      railway deployment list \
        --project "$RAILWAY_PROJECT_ID" \
        --service "$service_name" \
        --environment "$railway_environment" \
        --limit 10 \
        --json
    )"; then
      deployment_line="$(
        DEPLOYMENTS_JSON="$deployment_json" \
        DEPLOYMENT_ID="$deployment_id" \
        node -e 'const deployments = JSON.parse(process.env.DEPLOYMENTS_JSON || "[]"); const id = process.env.DEPLOYMENT_ID || ""; const deployment = deployments.find((candidate) => candidate.id === id); if (deployment) console.log([deployment.id, deployment.status, deployment.meta?.skippedReason || ""].join("\t"));'
      )"

      if [ -n "$deployment_line" ]; then
        IFS=$'\t' read -r deployment_id deployment_status skipped_reason <<< "$deployment_line"
        echo "Railway deployment $deployment_id is $deployment_status."

        case "$deployment_status" in
          SUCCESS)
            return 0
            ;;
          SKIPPED)
            echo "Railway skipped deployment: ${skipped_reason:-no reason provided}."
            if [[ -n "$deployment_component" && -n "$deployment_source_sha" \
              && "$verified_successful_sha" == "$deployment_source_sha" ]]; then
              echo "The durable deployment ledger already records this exact source revision as successful."
              deployment_was_skipped=1
              return 0
            fi
            echo "Refusing to infer source revision from current service health." >&2
            return 1
            ;;
          FAILED|CRASHED|REMOVED)
            print_deployment_logs "$service_name" "$deployment_id"
            return 1
            ;;
        esac
      else
        echo "Waiting for Railway deployment $deployment_id to appear..."
      fi
    else
      echo "Waiting for Railway deployment status..."
    fi

    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "Timed out waiting for Railway deployment."
      return 1
    fi

    sleep 10
  done
}

deploy_message="$(deploy_message_from_event)"
deploy_output=""
deployment_id=""

ensure_service_exists "$service_name"

deploy_output="$(
  railway up "$upload_root" \
  --path-as-root \
  --no-gitignore \
  --project "$RAILWAY_PROJECT_ID" \
  --service "$service_name" \
  --environment "$railway_environment" \
  --message "$deploy_message" \
  --detach \
  --json
)"
printf '%s\n' "$deploy_output"

deployment_id="$(
  DEPLOY_OUTPUT="$deploy_output" node -e '
const lines = (process.env.DEPLOY_OUTPUT || "").split(/\r?\n/).filter(Boolean);
for (const line of lines) {
  try {
    const parsed = JSON.parse(line);
    if (parsed && typeof parsed.deploymentId === "string" && parsed.deploymentId.length > 0) {
      process.stdout.write(parsed.deploymentId);
      process.exit(0);
    }
  } catch {}
}
process.exit(1);
'
)"

wait_for_deployment "$service_name" "$deployment_id"
if [[ "$defer_service_health" == "0" ]]; then
  if [[ "$deployment_was_skipped" == "1" ]]; then
    service_is_healthy "$service_name"
  else
    wait_for_service_health "$service_name" "$deployment_id"
  fi
fi
if [[ "$deployment_was_skipped" == "0" ]]; then
  record_deployment_evidence "$deployment_id"
fi
