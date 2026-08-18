#!/usr/bin/env bash
set -euo pipefail

: "${NORTHFLANK_API_TOKEN:?NORTHFLANK_API_TOKEN is required}"
: "${NORTHFLANK_PROJECT_ID:?NORTHFLANK_PROJECT_ID is required}"
: "${NORTHFLANK_DEPLOYMENT_PLAN:?NORTHFLANK_DEPLOYMENT_PLAN is required}"
: "${SCOPE_WORKFLOW_IMAGE:?SCOPE_WORKFLOW_IMAGE must be a pinned image digest}"

command -v jq >/dev/null || { echo "jq is required" >&2; exit 2; }

case "$SCOPE_WORKFLOW_IMAGE" in
  *@sha256:????????????????????????????????????????????????????????????????) ;;
  *) echo "SCOPE_WORKFLOW_IMAGE must end in a sha256 digest" >&2; exit 2 ;;
esac

payload="$(jq -cn \
  --arg plan "$NORTHFLANK_DEPLOYMENT_PLAN" \
  --arg image "$SCOPE_WORKFLOW_IMAGE" \
  --arg credentials "${NORTHFLANK_REGISTRY_CREDENTIALS_ID:-}" \
  '{
    name: "Scope cloud runs",
    description: "Reusable Scope run-once execution primitive",
    billing: {deploymentPlan: $plan},
    deployment: {
      docker: {configType: "customEntrypoint", customEntrypoint: "/scope/bin/scope-runner-runtime"},
      storage: {ephemeralStorage: {storageSize: 20480}},
      external: ({imagePath: $image} + if $credentials == "" then {} else {credentials: $credentials} end)
    },
    settings: {backoffLimit: 0, runOnSourceChange: "never", activeDeadlineSeconds: 86400}
  }')"

response="$(curl --fail-with-body --silent --show-error \
  --request POST \
  --header "Authorization: Bearer $NORTHFLANK_API_TOKEN" \
  --header 'Content-Type: application/json' \
  --data "$payload" \
  "https://api.northflank.com/v1/projects/$NORTHFLANK_PROJECT_ID/jobs")"

printf '%s\n' "$response"
