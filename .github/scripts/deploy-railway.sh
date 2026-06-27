#!/usr/bin/env bash
set -euo pipefail

service_name="${1:?usage: deploy-railway.sh <service-name> <upload-root>}"
upload_root="${2:?usage: deploy-railway.sh <service-name> <upload-root>}"

if [ -z "${RAILWAY_TOKEN:-}" ]; then
  echo "Set the RAILWAY_TOKEN repository secret before deploying ${service_name}."
  exit 1
fi

if [ -z "${RAILWAY_PROJECT_ID:-}" ]; then
  echo "Set the RAILWAY_PROJECT_ID repository secret before deploying ${service_name}."
  exit 1
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

wait_for_deployment() {
  local service_name="$1"
  local message="$2"
  local started_at="$3"
  local deadline=$((SECONDS + 900))
  local deployment_json
  local deployment_line
  local deployment_id
  local deployment_status
  local skipped_reason

  while true; do
    if deployment_json="$(
      railway deployment list \
        --project "$RAILWAY_PROJECT_ID" \
        --service "$service_name" \
        --environment production \
        --limit 10 \
        --json
    )"; then
      deployment_line="$(
        DEPLOYMENTS_JSON="$deployment_json" \
        DEPLOY_MESSAGE="$message" \
        DEPLOY_STARTED_AT="$started_at" \
        node -e 'const deployments = JSON.parse(process.env.DEPLOYMENTS_JSON || "[]"); const message = process.env.DEPLOY_MESSAGE || ""; const startedAt = Date.parse(process.env.DEPLOY_STARTED_AT || "1970-01-01T00:00:00Z") - 30000; const deployment = deployments.find((candidate) => { const createdAt = Date.parse(candidate.createdAt || ""); return candidate.meta?.cliMessage === message && Number.isFinite(createdAt) && createdAt >= startedAt; }); if (deployment) console.log([deployment.id, deployment.status, deployment.meta?.skippedReason || ""].join("\t"));'
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
            return 0
            ;;
          FAILED|CRASHED|REMOVED)
            return 1
            ;;
        esac
      else
        echo "Waiting for Railway deployment to appear..."
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
deploy_started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

railway up "$upload_root" \
  --path-as-root \
  --no-gitignore \
  --project "$RAILWAY_PROJECT_ID" \
  --service "$service_name" \
  --environment production \
  --message "$deploy_message" \
  --detach \
  --json

wait_for_deployment "$service_name" "$deploy_message" "$deploy_started_at"
