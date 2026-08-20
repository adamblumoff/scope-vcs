#!/usr/bin/env bash
set -euo pipefail

action="${1:?usage: scale-staging-writers.sh <close|open>}"
manifest_path="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"

if [[ -z "${RAILWAY_API_TOKEN:-}" || -n "${RAILWAY_TOKEN:-}" ]]; then
  echo "Staging writer scaling requires only RAILWAY_API_TOKEN." >&2
  exit 1
fi

project_id="$(jq -er '.railway.projectId' "$manifest_path")"
production_environment_id="$(jq -er '.railway.environmentId' "$manifest_path")"
staging_environment_id="$(jq -er '.railway.staging.environmentId' "$manifest_path")"
staging_region="$(jq -er '.railway.regionId' "$manifest_path")"
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

scale_service() {
  railway service scale "${railway_scope[@]}" --service "$1" \
    "$staging_region=$2" --json >/dev/null
}

wait_for_replica_state() {
  local service="$1"
  local expected_running="$2"
  local deadline=$((SECONDS + 300))
  local state status configured running crashed

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

case "$action" in
  close)
    scale_service "$api_service" 0
    scale_service "$worker_service" 0
    wait_for_replica_state "$api_service" 0
    wait_for_replica_state "$worker_service" 0
    ;;
  open)
    scale_service "$worker_service" 1
    wait_for_replica_state "$worker_service" 1
    scale_service "$api_service" 1
    wait_for_replica_state "$api_service" 1
    ;;
  *)
    echo "usage: scale-staging-writers.sh <close|open>" >&2
    exit 2
    ;;
esac
