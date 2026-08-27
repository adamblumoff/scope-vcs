#!/usr/bin/env bash
set -euo pipefail

api_upload_root="${1:?usage: deploy-backend-railway.sh <api-upload-root> <worker-upload-root> <cache-upload-root>}"
worker_upload_root="${2:?usage: deploy-backend-railway.sh <api-upload-root> <worker-upload-root> <cache-upload-root>}"
cache_upload_root="${3:?usage: deploy-backend-railway.sh <api-upload-root> <worker-upload-root> <cache-upload-root>}"
maintenance_binary="${SCOPE_MAINTENANCE_BINARY:-./target/release/scope-maintenance}"
environment="${SCOPE_RAILWAY_ENVIRONMENT_ID:?SCOPE_RAILWAY_ENVIRONMENT_ID is required}"
api_service="${SCOPE_RAILWAY_API_SERVICE_ID:?SCOPE_RAILWAY_API_SERVICE_ID is required}"
worker_service="${SCOPE_RAILWAY_WORKER_SERVICE_ID:?SCOPE_RAILWAY_WORKER_SERVICE_ID is required}"
cache_service="${SCOPE_RAILWAY_CACHE_SERVICE_ID:?SCOPE_RAILWAY_CACHE_SERVICE_ID is required}"
database_service="${SCOPE_RAILWAY_DATABASE_SERVICE_ID:?SCOPE_RAILWAY_DATABASE_SERVICE_ID is required}"
api_region="${SCOPE_RAILWAY_API_REGION_ID:?SCOPE_RAILWAY_API_REGION_ID is required}"
worker_region="${SCOPE_RAILWAY_WORKER_REGION_ID:?SCOPE_RAILWAY_WORKER_REGION_ID is required}"
recover_closed_cutover="${SCOPE_RECOVER_CLOSED_CUTOVER:-0}"
deploy_cache_requested="${SCOPE_DEPLOY_CACHE:-1}"
deploy_worker_requested="${SCOPE_DEPLOY_WORKER:-1}"
deploy_api_requested="${SCOPE_DEPLOY_API:-1}"
successful_deployment_revisions="${SCOPE_SUCCESSFUL_DEPLOYMENT_REVISIONS:-}"
[[ -n "$successful_deployment_revisions" ]] || successful_deployment_revisions='{}'
deployment_evidence_path="${SCOPE_DEPLOYMENT_EVIDENCE_PATH:-}"
pending_evidence_path=""
if [[ -n "$deployment_evidence_path" ]]; then
  pending_evidence_path="${deployment_evidence_path}.pending.$$"
  rm -f -- "$pending_evidence_path"
fi

for deployment_flag in deploy_cache_requested deploy_worker_requested deploy_api_requested; do
  if [[ "${!deployment_flag}" != "0" && "${!deployment_flag}" != "1" ]]; then
    echo "${deployment_flag} must be 0 or 1." >&2
    exit 2
  fi
done
if [[ "$deploy_cache_requested" == "0" && "$deploy_worker_requested" == "0" \
  && "$deploy_api_requested" == "0" ]]; then
  echo "At least one backend service must be selected for deployment." >&2
  exit 2
fi

if [[ -z "${RAILWAY_TOKEN:-}" || -z "${RAILWAY_API_TOKEN:-}" || -z "${RAILWAY_PROJECT_ID:-}" ]]; then
  echo "RAILWAY_TOKEN, RAILWAY_API_TOKEN, and RAILWAY_PROJECT_ID are required for backend deployment." >&2
  exit 1
fi
if [[ ! -x "$maintenance_binary" ]]; then
  echo "Maintenance binary is not executable: $maintenance_binary" >&2
  exit 1
fi

# The Railway CLI accepts project tokens but currently rejects workspace tokens. Keep the
# workspace token out of its environment and use it only for deployment control below.
railway_api_token="$RAILWAY_API_TOKEN"
unset RAILWAY_API_TOKEN

railway_scope=(--project "$RAILWAY_PROJECT_ID" --environment "$environment")
cutover_committed=0
api_closed=0
worker_closed=0
cache_closed=0

validate_production_target() {
  local status_json services_json environment_config_json
  status_json="$(railway status "${railway_scope[@]}" --json)"
  services_json="$(railway service list "${railway_scope[@]}" --json)"
  environment_config_json="$(railway environment config --environment "$environment" --json)"
  # The JavaScript template literals are evaluated by Node.
  # shellcheck disable=SC2016
  RAILWAY_STATUS_JSON="$status_json" \
    RAILWAY_SERVICES_JSON="$services_json" \
    RAILWAY_ENVIRONMENT_CONFIG_JSON="$environment_config_json" \
    EXPECTED_PROJECT_ID="$RAILWAY_PROJECT_ID" \
    EXPECTED_ENVIRONMENT_ID="$environment" \
    EXPECTED_API_SERVICE_ID="$api_service" \
    EXPECTED_WORKER_SERVICE_ID="$worker_service" \
    EXPECTED_CACHE_SERVICE_ID="$cache_service" \
    EXPECTED_DATABASE_SERVICE_ID="$database_service" \
    EXPECTED_API_REGION="$api_region" \
    EXPECTED_WORKER_REGION="$worker_region" \
    node -e '
const status = JSON.parse(process.env.RAILWAY_STATUS_JSON || "{}");
const serviceStates = JSON.parse(process.env.RAILWAY_SERVICES_JSON || "[]");
const environmentConfig = JSON.parse(process.env.RAILWAY_ENVIRONMENT_CONFIG_JSON || "{}");
const fail = (message) => {
  console.error(`Refusing backend deployment: ${message}.`);
  process.exit(1);
};
const expectedServices = new Map([
  [process.env.EXPECTED_API_SERVICE_ID, "scope-api"],
  [process.env.EXPECTED_WORKER_SERVICE_ID, "scope-worker"],
  [process.env.EXPECTED_CACHE_SERVICE_ID, "scope-cache-service"],
  [process.env.EXPECTED_DATABASE_SERVICE_ID, "scope-postgres"],
]);
const environments = status.environments?.edges?.map(({node}) => node) || [];
const services = status.services?.edges?.map(({node}) => node) || [];
if (status.id !== process.env.EXPECTED_PROJECT_ID) fail("Railway project ID does not match the reviewed target");
if (!environments.some(({id, name}) => id === process.env.EXPECTED_ENVIRONMENT_ID && name === "production")) {
  fail("Railway production environment does not match the reviewed target");
}
for (const [id, name] of expectedServices) {
  if (!services.some((service) => service.id === id && service.name === name)) {
    fail(`Railway service ${name} does not match the reviewed target`);
  }
}
for (const [id, name, expectedRegion] of [
  [process.env.EXPECTED_API_SERVICE_ID, "scope-api", process.env.EXPECTED_API_REGION],
  [process.env.EXPECTED_WORKER_SERVICE_ID, "scope-worker", process.env.EXPECTED_WORKER_REGION],
]) {
  const service = serviceStates.find((candidate) => candidate.id === id);
  if (!service) fail(`Railway service ${name} has no production state`);
  const configured = service.replicas?.configured || 0;
  if (configured > 0 && !service.regions?.some(
    (region) => region.name === expectedRegion && region.configured === configured,
  )) {
    fail(`Railway service ${name} is not running in reviewed region ${expectedRegion}`);
  }
  const storedRegions = environmentConfig.services?.[id]?.deploy?.multiRegionConfig || {};
  const activeStoredRegions = Object.entries(storedRegions).filter(
    ([, config]) => config && Number(config.numReplicas) > 0,
  );
  if (configured > 0 && !(
    activeStoredRegions.length === 1 &&
    activeStoredRegions[0][0] === expectedRegion &&
    Number(activeStoredRegions[0][1].numReplicas) === configured
  )) {
    fail(`Railway service ${name} stored region config does not match reviewed region ${expectedRegion}`);
  }
}
'
}

maintenance() {
  # `railway run` executes on this CI host, so the database service's public proxy is required.
  # The child shell expands the Railway-injected database URL and command arguments.
  # shellcheck disable=SC2016
  railway run "${railway_scope[@]}" --service "$database_service" --no-local -- \
    sh -c 'DATABASE_URL="$DATABASE_PUBLIC_URL" exec "$@"' \
    scope-maintenance "$maintenance_binary" "$1"
}

maintenance_read() {
  local command="$1"
  local attempt output
  for attempt in 1 2 3; do
    if output="$(maintenance "$command")"; then
      printf '%s\n' "$output"
      return 0
    fi
    if [[ "$attempt" -lt 3 ]]; then
      echo "Maintenance $command failed; retrying read-only database access." >&2
      sleep 2
    fi
  done
  return 1
}

backfill_repository_snapshots() {
  local database_public_url maintenance_data_dir result backfill_command
  database_public_url="$(
    railway variable list "${railway_scope[@]}" --service "$database_service" --json |
      jq -er '.DATABASE_PUBLIC_URL | strings | select(length > 0)'
  )"
  maintenance_data_dir="$(mktemp -d "${RUNNER_TEMP:-/tmp}/scope-repository-snapshot-backfill.XXXXXX")"
  result=0
  for backfill_command in backfill-landing-files backfill-workflow-catalogs; do
    SCOPE_MAINTENANCE_DATABASE_URL="$database_public_url" \
      SCOPE_MAINTENANCE_DATA_DIR="$maintenance_data_dir" \
      railway run "${railway_scope[@]}" --service "$api_service" --no-local -- \
        sh -c 'DATABASE_URL="$SCOPE_MAINTENANCE_DATABASE_URL" SCOPE_DATA_DIR="$SCOPE_MAINTENANCE_DATA_DIR" exec "$@"' \
        scope-maintenance "$maintenance_binary" "$backfill_command" || {
          result=$?
          break
        }
  done
  rm -rf -- "$maintenance_data_dir"
  return "$result"
}

wait_for_writer_fence() {
  local grace_seconds="${SCOPE_WRITER_FENCE_GRACE_SECONDS:-10}"
  local grace_deadline=$((SECONDS + grace_seconds))
  local deadline=$((SECONDS + 120))
  local drained=0
  while (( SECONDS < deadline )); do
    if maintenance fence; then
      return 0
    fi
    if [[ "$drained" == "0" && "$SECONDS" -ge "$grace_deadline" ]]; then
      maintenance drain-writers
      drained=1
    fi
    echo "Metadata writers are still draining; retrying the fence probe." >&2
    sleep 2
  done
  echo "Timed out waiting for metadata writers to release the database fence." >&2
  return 1
}

plan_requires_maintenance() {
  PLAN_JSON="$1" node -e '
const plan = JSON.parse(process.env.PLAN_JSON || "{}");
if (!Array.isArray(plan.pending)) process.exit(2);
process.exit(plan.pending.some((item) => item.impact === "maintenance-required") ? 0 : 1);
'
}

plan_is_exact() {
  PLAN_JSON="$1" node -e '
const plan = JSON.parse(process.env.PLAN_JSON || "{}");
process.exit(plan.exact === true && Array.isArray(plan.pending) && plan.pending.length === 0 ? 0 : 1);
'
}

plans_have_same_ledger() {
  BEFORE_PLAN_JSON="$1" AFTER_PLAN_JSON="$2" node -e '
const before = JSON.parse(process.env.BEFORE_PLAN_JSON || "{}");
const after = JSON.parse(process.env.AFTER_PLAN_JSON || "{}");
const ledger = (plan) => Array.isArray(plan.pending)
  ? plan.pending.map(({name, impact}) => ({name, impact}))
  : null;
const beforeLedger = ledger(before);
const afterLedger = ledger(after);
process.exit(
  before.exact === after.exact &&
  beforeLedger !== null &&
  afterLedger !== null &&
  JSON.stringify(beforeLedger) === JSON.stringify(afterLedger)
    ? 0
    : 1,
);
'
}

service_state_line() {
  local service_name="$1"
  local services_json
  services_json="$(railway service list "${railway_scope[@]}" --json)"
  SERVICES_JSON="$services_json" SERVICE_NAME="$service_name" node -e '
const services = JSON.parse(process.env.SERVICES_JSON || "[]");
const target = process.env.SERVICE_NAME;
const service = services.find((item) => item.id === target || item.name === target);
if (!service) process.exit(1);
const replicas = service.replicas || {};
console.log([
  service.status || "",
  replicas.running || 0,
  replicas.crashed || 0,
  replicas.configured || 0,
  service.deploymentStopped === true ? "1" : "0",
  service.deploymentId || "",
].join("\t"));
'
}

wait_for_service_health() {
  local service_name="$1"
  local deadline=$((SECONDS + 600))
  local line status running crashed configured stopped id
  while (( SECONDS < deadline )); do
    line="$(service_state_line "$service_name" || true)"
    if [[ -n "$line" ]]; then
      IFS=$'\t' read -r status running crashed configured stopped id <<< "$line"
      if [[ "$status" == "SUCCESS" && "$stopped" == "0" && "$configured" -gt 0 \
        && "$running" == "$configured" && "${crashed:-0}" == "0" ]]; then
        return 0
      fi
    fi
    sleep 10
  done
  echo "Timed out waiting for $service_name to reach its configured healthy replica count." >&2
  return 1
}

service_is_healthy() {
  local line status running crashed configured stopped id
  line="$(service_state_line "$1")"
  IFS=$'\t' read -r status running crashed configured stopped id <<< "$line"
  [[ "$status" == "SUCCESS" && "$stopped" == "0" && "$configured" -gt 0 \
    && "$running" == "$configured" && "${crashed:-0}" == "0" ]]
}

running_replicas() {
  local line status running crashed configured stopped id
  line="$(service_state_line "$1")"
  IFS=$'\t' read -r status running crashed configured stopped id <<< "$line"
  printf '%s\n' "$running"
}

service_has_deployment_history() {
  local deployments_json
  deployments_json="$(railway deployment list "${railway_scope[@]}" --service "$1" --limit 1 --json)"
  DEPLOYMENTS_JSON="$deployments_json" node -e '
const deployments = JSON.parse(process.env.DEPLOYMENTS_JSON || "[]");
process.exit(Array.isArray(deployments) && deployments.length > 0 ? 0 : 1);
'
}

deployment_id() {
  local line status running crashed configured stopped id
  line="$(service_state_line "$1")"
  IFS=$'\t' read -r status running crashed configured stopped id <<< "$line"
  if [[ -z "$id" ]]; then
    echo "Railway service $1 has no active deployment to control." >&2
    return 1
  fi
  printf '%s\n' "$id"
}

deployment_action() {
  local action="$1"
  local service_name="$2"
  local id request response
  id="$(deployment_id "$service_name")"
  request="$(
    DEPLOYMENT_ACTION="$action" \
      DEPLOYMENT_ID="$id" \
      node -e '
const action = process.env.DEPLOYMENT_ACTION;
if (action !== "Stop" && action !== "Restart") process.exit(2);
console.log(JSON.stringify({
  query: `mutation deployment${action}($id: String!) {
    deployment${action}(id: $id)
  }`,
  variables: {id: process.env.DEPLOYMENT_ID},
}));
'
  )"
  response="$(
    printf 'Authorization: Bearer %s\nContent-Type: application/json\n' "$railway_api_token" \
      | curl --silent --show-error --fail-with-body \
        --request POST \
        --url https://backboard.railway.com/graphql/v2 \
        --header @- \
        --data-binary "$request"
  )"
  RAILWAY_GRAPHQL_RESPONSE="$response" DEPLOYMENT_ACTION="$action" node -e '
const response = JSON.parse(process.env.RAILWAY_GRAPHQL_RESPONSE || "{}");
const field = `deployment${process.env.DEPLOYMENT_ACTION}`;
if (response.data?.[field] !== true) {
  const messages = Array.isArray(response.errors)
    ? response.errors.map(({message}) => message).filter(Boolean).join("; ")
    : "";
  console.error(`Railway ${field} mutation failed${messages ? `: ${messages}` : "."}`);
  process.exit(1);
}
'
}

restart_service() {
  deployment_action Restart "$1"
  wait_for_service_health "$1"
}

quiesce_writers() {
  if [[ "$api_closed" == "0" ]]; then
    if [[ "$cutover_committed" == "0" ]] && ! service_is_healthy "$api_service"; then
      echo "Refusing maintenance because $api_service is not healthy before shutdown." >&2
      return 1
    fi
    deployment_action Stop "$api_service"
    api_closed=1
  fi
  if [[ "$worker_closed" == "0" ]]; then
    if [[ "$cutover_committed" == "0" ]] && ! service_is_healthy "$worker_service"; then
      echo "Refusing maintenance because $worker_service is not healthy before shutdown." >&2
      return 1
    fi
    deployment_action Stop "$worker_service"
    worker_closed=1
  fi
  if [[ "$cache_closed" == "0" ]]; then
    if [[ "$cutover_committed" == "0" ]] && ! service_is_healthy "$cache_service"; then
      echo "Refusing maintenance because $cache_service is not healthy before shutdown." >&2
      return 1
    fi
    deployment_action Stop "$cache_service"
    cache_closed=1
  fi
  # Railway's replica counts can remain stale after a successful stop mutation. The database
  # fence is the authoritative proof that every metadata writer has actually quiesced.
  wait_for_writer_fence
}

restore_old_release() {
  echo "Migration did not commit; restoring the previous metadata-writer deployments." >&2
  if [[ "$cache_closed" == "1" ]]; then
    restart_service "$cache_service"
    cache_closed=0
  fi
  if [[ "$worker_closed" == "1" ]]; then
    restart_service "$worker_service"
    worker_closed=0
  fi
  if [[ "$api_closed" == "1" ]]; then
    restart_service "$api_service"
    api_closed=0
  fi
}

successful_revision() {
  COMPONENT="$1" REVISIONS="$successful_deployment_revisions" node -e '
const revisions = JSON.parse(process.env.REVISIONS || "{}");
process.stdout.write(revisions[process.env.COMPONENT] || "");
'
}

deploy_release() {
  local component="$1"
  local service_name="$2"
  local upload_root="$3"
  local verified_sha
  verified_sha="$(successful_revision "$component")"
  SCOPE_DEPLOYMENT_COMPONENT="$component" \
    SCOPE_DEPLOYMENT_EVIDENCE_PATH="$pending_evidence_path" \
    SCOPE_VERIFIED_SUCCESSFUL_SHA="$verified_sha" \
    bash .github/scripts/deploy-railway.sh "$service_name" "$upload_root"
}

activate_release() {
  local component="$1"
  local service_name="$2"
  local upload_root="$3"
  deploy_release "$component" "$service_name" "$upload_root"
  if [[ "$(running_replicas "$service_name")" == "0" ]]; then
    restart_service "$service_name"
  else
    wait_for_service_health "$service_name"
  fi
}

promote_pending_evidence() {
  [[ -n "$deployment_evidence_path" && -s "$pending_evidence_path" ]] || return 0
  FINAL_EVIDENCE_PATH="$deployment_evidence_path" \
    PENDING_EVIDENCE_PATH="$pending_evidence_path" \
    node -e '
const { appendFileSync, readFileSync, unlinkSync } = require("node:fs");
appendFileSync(process.env.FINAL_EVIDENCE_PATH, readFileSync(process.env.PENDING_EVIDENCE_PATH));
unlinkSync(process.env.PENDING_EVIDENCE_PATH);
'
}

discard_pending_evidence() {
  [[ -z "$pending_evidence_path" ]] || rm -f -- "$pending_evidence_path"
}

deploy_cache_release() {
  activate_release cache "$cache_service" "$cache_upload_root"
  promote_pending_evidence
}

deploy_selected_releases() {
  if [[ "$deploy_cache_requested" == "1" ]]; then
    deploy_cache_release
  fi
  if [[ "$deploy_worker_requested" == "1" ]]; then
    deploy_release worker "$worker_service" "$worker_upload_root"
    promote_pending_evidence
  fi
  if [[ "$deploy_api_requested" == "1" ]]; then
    deploy_release api "$api_service" "$api_upload_root"
    promote_pending_evidence
  fi
}

deploy_and_reopen() {
  maintenance_read verify
  backfill_repository_snapshots
  deploy_cache_release
  cache_closed=0
  activate_release worker "$worker_service" "$worker_upload_root"
  worker_closed=0
  activate_release api "$api_service" "$api_upload_root"
  api_closed=0
  # Worker and API form one cutover. Publish their evidence only after both writers are healthy
  # so the durable ledger cannot claim a deployment that the failure trap subsequently closes.
  promote_pending_evidence
}

leave_failure_state() {
  local exit_status="$1"
  trap - EXIT
  if [[ "$exit_status" -ne 0 ]]; then
    if [[ "$cutover_committed" == "0" \
      && ( "$api_closed" == "1" || "$worker_closed" == "1" || "$cache_closed" == "1" ) ]]; then
      restore_old_release || echo "Failed to restore the previous release; writers remain closed." >&2
    elif [[ "$cutover_committed" == "1" ]]; then
      quiesce_writers || echo "Failed to re-close metadata writers after the committed cutover." >&2
      echo "Migration committed; writers remain closed. Rerun this workflow to finish the forward-only deployment." >&2
    fi
  fi
  discard_pending_evidence
  exit "$exit_status"
}
trap 'leave_failure_state $?' EXIT

validate_production_target

plan_json="$(maintenance_read plan)"
set +e
plan_requires_maintenance "$plan_json"
plan_status=$?
set -e
case "$plan_status" in
  0) ;;
  1)
    api_running="$(running_replicas "$api_service")"
    worker_running="$(running_replicas "$worker_service")"
    cache_running="$(running_replicas "$cache_service")"
    if [[ "$api_running" == "0" && "$worker_running" == "0" && "$cache_running" == "0" ]]; then
      api_has_history=0
      worker_has_history=0
      cache_has_history=0
      service_has_deployment_history "$api_service" && api_has_history=1
      service_has_deployment_history "$worker_service" && worker_has_history=1
      service_has_deployment_history "$cache_service" && cache_has_history=1
      if [[ "$api_has_history" != "$worker_has_history" ]] \
        || [[ "$api_has_history" != "$cache_has_history" ]]; then
        echo "Closed writers have inconsistent deployment history." >&2
        exit 1
      fi
      if [[ "$api_has_history" == "1" && "$recover_closed_cutover" != "1" ]]; then
        echo "Writers are intentionally closed or await cutover recovery; rerun this workflow to authorize reopening." >&2
        exit 1
      fi
      api_closed=1
      worker_closed=1
      cache_closed=1
      cutover_committed=1
      deploy_and_reopen
      trap - EXIT
      discard_pending_evidence
      exit 0
    fi
    if [[ "$api_running" == "0" || "$worker_running" == "0" || "$cache_running" == "0" ]]; then
      echo "Metadata-writer replica state is inconsistent; refusing deployment." >&2
      exit 1
    fi
    maintenance_read verify
    deploy_selected_releases
    trap - EXIT
    discard_pending_evidence
    exit 0
    ;;
  *)
    echo "Maintenance plan output was invalid; refusing deployment." >&2
    exit 1
    ;;
esac

if ! service_is_healthy "$api_service" || ! service_is_healthy "$worker_service" \
  || ! service_is_healthy "$cache_service"; then
  echo "Maintenance cutover requires healthy metadata-writer deployments before closing writers." >&2
  exit 1
fi

quiesce_writers
maintenance validate-workflow-catalogs
if ! maintenance apply; then
  # Once apply starts, an error does not prove its transaction rolled back. Fail closed unless a
  # fresh ledger read positively proves that the pre-migration state is unchanged.
  cutover_committed=1
  recovery_plan="$(maintenance_read plan || true)"
  if [[ -n "$recovery_plan" ]] && plan_is_exact "$recovery_plan"; then
    cutover_committed=1
  elif [[ -n "$recovery_plan" ]] && plans_have_same_ledger "$plan_json" "$recovery_plan"; then
    cutover_committed=0
  fi
  exit 1
fi
cutover_committed=1

deploy_and_reopen
trap - EXIT
discard_pending_evidence
