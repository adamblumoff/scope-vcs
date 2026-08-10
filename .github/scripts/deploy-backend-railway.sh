#!/usr/bin/env bash
set -euo pipefail

api_upload_root="${1:?usage: deploy-backend-railway.sh <api-upload-root> <worker-upload-root>}"
worker_upload_root="${2:?usage: deploy-backend-railway.sh <api-upload-root> <worker-upload-root>}"
maintenance_binary="${SCOPE_MAINTENANCE_BINARY:-./target/release/scope-maintenance}"
workspace_id="${SCOPE_RAILWAY_WORKSPACE_ID:?SCOPE_RAILWAY_WORKSPACE_ID is required}"
environment="${SCOPE_RAILWAY_ENVIRONMENT_ID:?SCOPE_RAILWAY_ENVIRONMENT_ID is required}"
api_service="${SCOPE_RAILWAY_API_SERVICE_ID:?SCOPE_RAILWAY_API_SERVICE_ID is required}"
worker_service="${SCOPE_RAILWAY_WORKER_SERVICE_ID:?SCOPE_RAILWAY_WORKER_SERVICE_ID is required}"
database_service="${SCOPE_RAILWAY_DATABASE_SERVICE_ID:?SCOPE_RAILWAY_DATABASE_SERVICE_ID is required}"
recover_closed_cutover="${SCOPE_RECOVER_CLOSED_CUTOVER:-0}"

if [[ -z "${RAILWAY_API_TOKEN:-}" || -z "${RAILWAY_PROJECT_ID:-}" ]]; then
  echo "RAILWAY_API_TOKEN and RAILWAY_PROJECT_ID are required for backend deployment." >&2
  exit 1
fi
if [[ -n "${RAILWAY_TOKEN:-}" ]]; then
  echo "RAILWAY_TOKEN must not be set for backend deployment; use only RAILWAY_API_TOKEN." >&2
  exit 1
fi
if [[ ! -x "$maintenance_binary" ]]; then
  echo "Maintenance binary is not executable: $maintenance_binary" >&2
  exit 1
fi

railway_scope=(--project "$RAILWAY_PROJECT_ID" --environment "$environment")
cutover_committed=0
api_closed=0
worker_closed=0
api_topology=()
worker_topology=()

validate_production_target() {
  local status_json
  status_json="$(railway status "${railway_scope[@]}" --json)"
  RAILWAY_STATUS_JSON="$status_json" \
    EXPECTED_PROJECT_ID="$RAILWAY_PROJECT_ID" \
    EXPECTED_WORKSPACE_ID="$workspace_id" \
    EXPECTED_ENVIRONMENT_ID="$environment" \
    EXPECTED_API_SERVICE_ID="$api_service" \
    EXPECTED_WORKER_SERVICE_ID="$worker_service" \
    EXPECTED_DATABASE_SERVICE_ID="$database_service" \
    node -e '
const status = JSON.parse(process.env.RAILWAY_STATUS_JSON || "{}");
const fail = (message) => {
  console.error(`Refusing backend deployment: ${message}.`);
  process.exit(1);
};
const expectedServices = new Map([
  [process.env.EXPECTED_API_SERVICE_ID, "scope-api"],
  [process.env.EXPECTED_WORKER_SERVICE_ID, "scope-worker"],
  [process.env.EXPECTED_DATABASE_SERVICE_ID, "scope-postgres"],
]);
const environments = status.environments?.edges?.map(({node}) => node) || [];
const services = status.services?.edges?.map(({node}) => node) || [];
if (status.id !== process.env.EXPECTED_PROJECT_ID) fail("Railway project ID does not match the reviewed target");
if (status.workspaceId !== process.env.EXPECTED_WORKSPACE_ID) fail("Railway workspace ID does not match the reviewed target");
if (!environments.some(({id, name}) => id === process.env.EXPECTED_ENVIRONMENT_ID && name === "production")) {
  fail("Railway production environment does not match the reviewed target");
}
for (const [id, name] of expectedServices) {
  if (!services.some((service) => service.id === id && service.name === name)) {
    fail(`Railway service ${name} does not match the reviewed target`);
  }
}
'
}

maintenance() {
  # `railway run` executes on this CI host, so the database service's public proxy is required.
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

deployment_topology() {
  local service_name="$1"
  local deployments_json
  deployments_json="$(
    railway deployment list "${railway_scope[@]}" \
      --service "$service_name" --limit 20 --json
  )"
  DEPLOYMENTS_JSON="$deployments_json" node -e '
const deployments = JSON.parse(process.env.DEPLOYMENTS_JSON || "[]");
for (const deployment of deployments) {
  if (deployment.status !== "SUCCESS") continue;
  const deploy = deployment.meta?.serviceManifest?.deploy;
  if (!deploy) continue;
  let regions = deploy.multiRegionConfig;
  if (!regions && typeof deploy.region === "string") {
    regions = {[deploy.region]: {numReplicas: deploy.numReplicas ?? 1}};
  }
  if (!regions || typeof regions !== "object") continue;
  const entries = Object.entries(regions)
    .map(([region, config]) => [region, config?.numReplicas ?? 0])
    .filter(([, replicas]) => Number.isInteger(replicas) && replicas > 0)
    .sort(([left], [right]) => left.localeCompare(right));
  if (entries.length === 0) continue;
  for (const [region, replicas] of entries) console.log(`${region}=${replicas}`);
  process.exit(0);
}
process.exit(1);
'
}

configured_topology() {
  local variable_name="$1"
  local configured="${!variable_name:-}"
  local assignment
  for assignment in $configured; do
    if [[ ! "$assignment" =~ ^[A-Za-z0-9_-]+=[1-9][0-9]*$ ]]; then
      echo "$variable_name must contain positive region=replica assignments." >&2
      return 1
    fi
    printf '%s\n' "$assignment"
  done
}

zero_topology() {
  local assignment
  for assignment in "$@"; do
    printf '%s=0\n' "${assignment%%=*}"
  done
}

service_replica_line() {
  local service_name="$1"
  local services_json
  services_json="$(railway service list "${railway_scope[@]}" --json)"
  SERVICES_JSON="$services_json" SERVICE_NAME="$service_name" node -e '
const services = JSON.parse(process.env.SERVICES_JSON || "[]");
const service = services.find((item) => item.name === process.env.SERVICE_NAME);
if (!service) process.exit(1);
const replicas = service.replicas || {};
console.log([service.status || "", replicas.running || 0, replicas.crashed || 0].join("\t"));
'
}

wait_for_replica_state() {
  local service_name="$1"
  local expected_running="$2"
  local deadline=$((SECONDS + 600))
  local line status running crashed
  while (( SECONDS < deadline )); do
    line="$(service_replica_line "$service_name" || true)"
    if [[ -n "$line" ]]; then
      IFS=$'\t' read -r status running crashed <<< "$line"
      if [[ "$running" == "$expected_running" && "${crashed:-0}" == "0" ]]; then
        if [[ "$expected_running" == "0" || "$status" == "SUCCESS" ]]; then
          return 0
        fi
      fi
    fi
    sleep 10
  done
  echo "Timed out waiting for $service_name to reach $expected_running healthy replicas." >&2
  return 1
}

replica_total() {
  local total=0 assignment
  for assignment in "$@"; do
    total=$((total + ${assignment#*=}))
  done
  printf '%s\n' "$total"
}

running_replicas() {
  local line status running crashed
  line="$(service_replica_line "$1")"
  IFS=$'\t' read -r status running crashed <<< "$line"
  printf '%s\n' "$running"
}

scale_service() {
  local service_name="$1"
  shift
  railway service scale "${railway_scope[@]}" --service "$service_name" --json "$@"
}

quiesce_writers() {
  local api_zero worker_zero
  mapfile -t api_zero < <(zero_topology "${api_topology[@]}")
  mapfile -t worker_zero < <(zero_topology "${worker_topology[@]}")
  if [[ "$api_closed" == "0" ]]; then
    scale_service "$api_service" "${api_zero[@]}"
    api_closed=1
    wait_for_replica_state "$api_service" 0
  fi
  if [[ "$worker_closed" == "0" ]]; then
    scale_service "$worker_service" "${worker_zero[@]}"
    worker_closed=1
    wait_for_replica_state "$worker_service" 0
  fi
}

restore_old_release() {
  echo "Migration did not commit; restoring the previous worker and API topology." >&2
  if [[ "$worker_closed" == "1" ]]; then
    scale_service "$worker_service" "${worker_topology[@]}"
    worker_closed=0
    wait_for_replica_state "$worker_service" "$(replica_total "${worker_topology[@]}")"
  fi
  if [[ "$api_closed" == "1" ]]; then
    scale_service "$api_service" "${api_topology[@]}"
    api_closed=0
    wait_for_replica_state "$api_service" "$(replica_total "${api_topology[@]}")"
  fi
}

deploy_and_reopen() {
  bash .github/scripts/deploy-railway.sh "$api_service" "$api_upload_root" stopped
  bash .github/scripts/deploy-railway.sh "$worker_service" "$worker_upload_root" stopped
  maintenance_read verify
  scale_service "$worker_service" "${worker_topology[@]}"
  worker_closed=0
  wait_for_replica_state "$worker_service" "$(replica_total "${worker_topology[@]}")"
  scale_service "$api_service" "${api_topology[@]}"
  api_closed=0
  wait_for_replica_state "$api_service" "$(replica_total "${api_topology[@]}")"
}

leave_failure_state() {
  local exit_status="$1"
  trap - EXIT
  if [[ "$exit_status" -ne 0 && ( "$api_closed" == "1" || "$worker_closed" == "1" ) ]]; then
    if [[ "$cutover_committed" == "0" ]]; then
      restore_old_release || echo "Failed to restore the previous release; writers remain closed." >&2
    else
      quiesce_writers || echo "Failed to re-close both writers after the committed cutover." >&2
      echo "Migration committed; writers remain closed. Rerun this workflow to finish the forward-only deployment." >&2
    fi
  fi
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
    if [[ "$api_running" == "0" && "$worker_running" == "0" ]]; then
      mapfile -t api_topology < <(deployment_topology "$api_service")
      mapfile -t worker_topology < <(deployment_topology "$worker_service")
      if [[ "${#api_topology[@]}" == "0" && "${#worker_topology[@]}" == "0" ]]; then
        mapfile -t api_topology < <(configured_topology SCOPE_API_TOPOLOGY)
        mapfile -t worker_topology < <(configured_topology SCOPE_WORKER_TOPOLOGY)
        if [[ "${#api_topology[@]}" == "0" || "${#worker_topology[@]}" == "0" ]]; then
          echo "Initial deployment requires explicit API and worker topology." >&2
          exit 1
        fi
      elif [[ "${#api_topology[@]}" == "0" || "${#worker_topology[@]}" == "0" ]]; then
        echo "Closed writers have inconsistent deployment topology history." >&2
        exit 1
      elif [[ "$recover_closed_cutover" != "1" ]]; then
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
    bash .github/scripts/deploy-railway.sh "$api_service" "$api_upload_root"
    maintenance_read verify
    bash .github/scripts/deploy-railway.sh "$worker_service" "$worker_upload_root"
    trap - EXIT
    exit 0
    ;;
  *)
    echo "Maintenance plan output was invalid; refusing deployment." >&2
    exit 1
    ;;
esac

mapfile -t api_topology < <(deployment_topology "$api_service")
mapfile -t worker_topology < <(deployment_topology "$worker_service")
if [[ "${#api_topology[@]}" == "0" && "${#worker_topology[@]}" == "0" ]]; then
  mapfile -t api_topology < <(configured_topology SCOPE_API_TOPOLOGY)
  mapfile -t worker_topology < <(configured_topology SCOPE_WORKER_TOPOLOGY)
fi
if [[ "${#api_topology[@]}" == "0" || "${#worker_topology[@]}" == "0" ]]; then
  echo "Could not read the current API and worker replica topology; refusing maintenance." >&2
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
