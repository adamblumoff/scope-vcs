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

if [[ -z "${RAILWAY_TOKEN:-}" || -z "${RAILWAY_API_TOKEN:-}" || -z "${RAILWAY_PROJECT_ID:-}" ]]; then
  echo "RAILWAY_TOKEN, RAILWAY_API_TOKEN, and RAILWAY_PROJECT_ID are required for backend deployment." >&2
  exit 1
fi
if [[ ! -x "$maintenance_binary" ]]; then
  echo "Maintenance binary is not executable: $maintenance_binary" >&2
  exit 1
fi

# The Railway CLI accepts project tokens but currently rejects workspace tokens. Keep the
# workspace token out of its environment and use it only for the scaling API mutation below.
railway_api_token="$RAILWAY_API_TOKEN"
unset RAILWAY_API_TOKEN

railway_scope=(--project "$RAILWAY_PROJECT_ID" --environment "$environment")
cutover_committed=0
api_closed=0
worker_closed=0
api_rollback_replicas=0
worker_rollback_replicas=0

validate_production_target() {
  local status_json services_json
  status_json="$(railway status "${railway_scope[@]}" --json)"
  services_json="$(railway service list "${railway_scope[@]}" --json)"
  # The JavaScript template literals are evaluated by Node.
  # shellcheck disable=SC2016
  RAILWAY_STATUS_JSON="$status_json" \
    RAILWAY_SERVICES_JSON="$services_json" \
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
].join("\t"));
'
}

wait_for_replica_state() {
  local service_name="$1"
  local expected_running="$2"
  local deadline=$((SECONDS + 600))
  local line status running crashed configured stopped
  while (( SECONDS < deadline )); do
    line="$(service_state_line "$service_name" || true)"
    if [[ -n "$line" ]]; then
      IFS=$'\t' read -r status running crashed configured stopped <<< "$line"
      if [[ "$running" == "$expected_running" && "${crashed:-0}" == "0" ]]; then
        if [[ "$expected_running" == "0" || ( "$status" == "SUCCESS" && "$stopped" == "0" ) ]]; then
          return 0
        fi
      fi
    fi
    sleep 10
  done
  echo "Timed out waiting for $service_name to reach $expected_running healthy replicas." >&2
  return 1
}

wait_for_service_health() {
  local service_name="$1"
  local deadline=$((SECONDS + 600))
  local line status running crashed configured stopped
  while (( SECONDS < deadline )); do
    line="$(service_state_line "$service_name" || true)"
    if [[ -n "$line" ]]; then
      IFS=$'\t' read -r status running crashed configured stopped <<< "$line"
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
  local line status running crashed configured stopped
  line="$(service_state_line "$1")"
  IFS=$'\t' read -r status running crashed configured stopped <<< "$line"
  [[ "$status" == "SUCCESS" && "$stopped" == "0" && "$configured" -gt 0 \
    && "$running" == "$configured" && "${crashed:-0}" == "0" ]]
}

running_replicas() {
  local line status running crashed configured stopped
  line="$(service_state_line "$1")"
  IFS=$'\t' read -r status running crashed configured stopped <<< "$line"
  printf '%s\n' "$running"
}

configured_replicas() {
  local line status running crashed configured stopped
  line="$(service_state_line "$1")"
  IFS=$'\t' read -r status running crashed configured stopped <<< "$line"
  printf '%s\n' "$configured"
}

configured_or_declared_replicas() {
  local service_name="$1"
  local variable_name="$2"
  local configured declared
  configured="$(configured_replicas "$service_name")"
  if [[ "$configured" -gt 0 ]]; then
    printf '%s\n' "$configured"
    return 0
  fi
  declared="${!variable_name:-}"
  if [[ ! "$declared" =~ ^[1-9][0-9]*$ ]]; then
    echo "$variable_name must declare a positive replica count when Railway has no deployment metadata." >&2
    return 1
  fi
  printf '%s\n' "$declared"
}

service_has_deployment_history() {
  local deployments_json
  deployments_json="$(railway deployment list "${railway_scope[@]}" --service "$1" --limit 1 --json)"
  DEPLOYMENTS_JSON="$deployments_json" node -e '
const deployments = JSON.parse(process.env.DEPLOYMENTS_JSON || "[]");
process.exit(Array.isArray(deployments) && deployments.length > 0 ? 0 : 1);
'
}

scale_service() {
  local service_name="$1"
  local replicas="$2"
  local region request response
  case "$service_name" in
    "$api_service") region="$api_region" ;;
    "$worker_service") region="$worker_region" ;;
    *)
      echo "No reviewed Railway region is configured for service $service_name." >&2
      return 1
      ;;
  esac
  request="$(
    SERVICE_ID="$service_name" \
      ENVIRONMENT_ID="$environment" \
      RAILWAY_REGION="$region" \
      REPLICA_COUNT="$replicas" \
      node -e '
const replicas = Number(process.env.REPLICA_COUNT);
if (!Number.isInteger(replicas) || replicas < 0) process.exit(2);
console.log(JSON.stringify({
  query: `mutation ScaleService($environmentId: String!, $patch: EnvironmentConfig!, $commitMessage: String) {
    environmentPatchCommit(environmentId: $environmentId, patch: $patch, commitMessage: $commitMessage)
  }`,
  variables: {
    environmentId: process.env.ENVIRONMENT_ID,
    patch: {
      services: {
        [process.env.SERVICE_ID]: {
          deploy: {
            // Railway represents zero replicas by removing the region from the environment config.
            multiRegionConfig: {
              [process.env.RAILWAY_REGION]: replicas === 0 ? null : {numReplicas: replicas},
            },
          },
        },
      },
    },
    commitMessage: `Scale service ${process.env.SERVICE_ID}`,
  },
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
  RAILWAY_GRAPHQL_RESPONSE="$response" node -e '
const response = JSON.parse(process.env.RAILWAY_GRAPHQL_RESPONSE || "{}");
if (typeof response.data?.environmentPatchCommit !== "string" || response.data.environmentPatchCommit.length === 0) {
  const messages = Array.isArray(response.errors)
    ? response.errors.map(({message}) => message).filter(Boolean).join("; ")
    : "";
  console.error(`Railway scaling mutation failed${messages ? `: ${messages}` : "."}`);
  process.exit(1);
}
'
}

quiesce_writers() {
  if [[ "$api_closed" == "0" ]]; then
    if [[ "$cutover_committed" == "0" ]] && ! service_is_healthy "$api_service"; then
      echo "Refusing maintenance because $api_service is not healthy before shutdown." >&2
      return 1
    fi
    scale_service "$api_service" 0
    api_closed=1
    wait_for_replica_state "$api_service" 0
  fi
  if [[ "$worker_closed" == "0" ]]; then
    if [[ "$cutover_committed" == "0" ]] && ! service_is_healthy "$worker_service"; then
      echo "Refusing maintenance because $worker_service is not healthy before shutdown." >&2
      return 1
    fi
    scale_service "$worker_service" 0
    worker_closed=1
    wait_for_replica_state "$worker_service" 0
  fi
}

restore_service() {
  local service_name="$1"
  local expected_replicas="$2"
  scale_service "$service_name" "$expected_replicas"
  wait_for_replica_state "$service_name" "$expected_replicas"
}

restore_old_release() {
  echo "Migration did not commit; restoring the previous worker and API deployments." >&2
  if [[ "$worker_closed" == "1" ]]; then
    restore_service "$worker_service" "$worker_rollback_replicas"
    worker_closed=0
  fi
  if [[ "$api_closed" == "1" ]]; then
    restore_service "$api_service" "$api_rollback_replicas"
    api_closed=0
  fi
}

deploy_cache() {
  bash .github/scripts/deploy-railway.sh "$cache_service" "$cache_upload_root"
  wait_for_service_health "$cache_service"
}

deploy_and_reopen() {
  maintenance_read verify
  deploy_cache
  bash .github/scripts/deploy-railway.sh "$worker_service" "$worker_upload_root" stopped
  scale_service "$worker_service" "$worker_rollback_replicas"
  wait_for_service_health "$worker_service"
  worker_closed=0
  bash .github/scripts/deploy-railway.sh "$api_service" "$api_upload_root" stopped
  scale_service "$api_service" "$api_rollback_replicas"
  wait_for_service_health "$api_service"
  api_closed=0
}

leave_failure_state() {
  local exit_status="$1"
  trap - EXIT
  if [[ "$exit_status" -ne 0 ]]; then
    if [[ "$cutover_committed" == "0" && ( "$api_closed" == "1" || "$worker_closed" == "1" ) ]]; then
      restore_old_release || echo "Failed to restore the previous release; writers remain closed." >&2
    elif [[ "$cutover_committed" == "1" ]]; then
      quiesce_writers || echo "Failed to re-close both writers after the committed cutover." >&2
      echo "Migration committed; writers remain closed. Rerun this workflow to finish the forward-only deployment." >&2
    fi
  fi
  exit "$exit_status"
}
trap 'leave_failure_state $?' EXIT

validate_production_target

api_rollback_replicas="$(
  configured_or_declared_replicas "$api_service" SCOPE_RAILWAY_API_REPLICAS
)"
worker_rollback_replicas="$(
  configured_or_declared_replicas "$worker_service" SCOPE_RAILWAY_WORKER_REPLICAS
)"

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
    if [[ "$api_running" == "0" && "$worker_running" == "0" ]]; then
      api_has_history=0
      worker_has_history=0
      service_has_deployment_history "$api_service" && api_has_history=1
      service_has_deployment_history "$worker_service" && worker_has_history=1
      if [[ "$api_has_history" != "$worker_has_history" ]]; then
        echo "Closed writers have inconsistent deployment history." >&2
        exit 1
      fi
      if [[ "$api_has_history" == "1" && "$recover_closed_cutover" != "1" ]]; then
        echo "Writers are intentionally closed or await cutover recovery; rerun this workflow to authorize reopening." >&2
        exit 1
      fi
      api_closed=1
      worker_closed=1
      cutover_committed=1
      deploy_and_reopen
      trap - EXIT
      exit 0
    fi
    if [[ "$api_running" == "0" || "$worker_running" == "0" ]]; then
      echo "API and worker replica state is inconsistent; refusing deployment." >&2
      exit 1
    fi
    maintenance_read verify
    deploy_cache
    bash .github/scripts/deploy-railway.sh "$worker_service" "$worker_upload_root"
    bash .github/scripts/deploy-railway.sh "$api_service" "$api_upload_root"
    trap - EXIT
    exit 0
    ;;
  *)
    echo "Maintenance plan output was invalid; refusing deployment." >&2
    exit 1
    ;;
esac

if ! service_is_healthy "$api_service" || ! service_is_healthy "$worker_service"; then
  echo "Maintenance cutover requires healthy API and worker deployments before closing writers." >&2
  exit 1
fi

quiesce_writers
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
