#!/usr/bin/env bash
set -euo pipefail

manifest_path="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
endpoint="https://backboard.railway.com/graphql/v2"

if [[ -z "${RAILWAY_API_TOKEN:-}" || -n "${RAILWAY_TOKEN:-}" ]]; then
  echo "Stopping staging writers requires only RAILWAY_API_TOKEN." >&2
  exit 1
fi

project_id="$(jq -er '.railway.projectId' "$manifest_path")"
production_environment_id="$(jq -er '.railway.environmentId' "$manifest_path")"
staging_environment_id="$(jq -er '.railway.staging.environmentId' "$manifest_path")"
worker_service="$(jq -er '.services.worker.id' "$manifest_path")"
api_service="$(jq -er '.services.api.id' "$manifest_path")"

if [[ "$staging_environment_id" == "$production_environment_id" ]]; then
  echo "Staging environment matches production." >&2
  exit 1
fi

railway_scope=(
  --project "$project_id"
  --environment "$staging_environment_id"
)

remove_deployment() {
  local deployment_id="$1"
  local request response
  request="$(
    jq -cn --arg id "$deployment_id" '{
      query: "mutation DeploymentRemove($id: String!) { deploymentRemove(id: $id) }",
      variables: {id: $id}
    }'
  )"
  response="$(
    curl --silent --show-error --fail-with-body \
      --request POST \
      --url "$endpoint" \
      --header "Authorization: Bearer $RAILWAY_API_TOKEN" \
      --header 'Content-Type: application/json' \
      --data-binary "$request"
  )"
  if jq -e '.errors | length > 0' >/dev/null 2>&1 <<< "$response"; then
    jq -r '.errors[]?.message // "Railway GraphQL request failed"' <<< "$response" >&2
    return 1
  fi
  jq -e '.data.deploymentRemove == true' >/dev/null <<< "$response"
}

stop_service() {
  local service="$1"
  local deployments deployment_id
  deployments="$(
    railway deployment list "${railway_scope[@]}" --service "$service" --limit 10 --json
  )"
  deployment_id="$(
    jq -er '
      first(
        .[] | select(
          .status != "REMOVED" and .status != "FAILED" and .status != "CRASHED" and
          .status != "SKIPPED"
        )
      ).id // empty
    ' <<< "$deployments" || true
  )"
  if [[ -n "$deployment_id" ]]; then
    remove_deployment "$deployment_id"
  fi
}

wait_until_stopped() {
  local service="$1"
  local deadline=$((SECONDS + 300))
  local running crashed

  while [[ "$SECONDS" -lt "$deadline" ]]; do
    IFS=$'\t' read -r running crashed < <(
      railway service list "${railway_scope[@]}" --json |
        jq -er --arg service "$service" '
          .[] | select(.id == $service) |
          [(.replicas.running // 0), (.replicas.crashed // 0)] | @tsv
        '
    )
    if [[ "$running" == "0" && "$crashed" == "0" ]]; then
      return 0
    fi
    sleep 5
  done

  echo "Timed out waiting for staging service $service to stop." >&2
  return 1
}

stop_service "$api_service"
stop_service "$worker_service"
wait_until_stopped "$api_service"
wait_until_stopped "$worker_service"
