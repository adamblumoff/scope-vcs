#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT
mkdir -p "$test_dir/bin" "$test_dir/api" "$test_dir/worker"

cat > "$test_dir/maintenance" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$0 $*" >> "$FAKE_RAILWAY_TRACE"
[[ "${DATABASE_URL:-}" == "postgres://public-database.test/scope" ]]

case "${1:-}" in
  plan)
    if [[ "${FAKE_FAIL_FIRST_PLAN:-0}" == "1" && ! -f "$FAKE_RAILWAY_STATE/first-plan-failed" ]]; then
      touch "$FAKE_RAILWAY_STATE/first-plan-failed"
      exit 1
    fi
    if [[ "${FAKE_FAIL_RECOVERY_PLAN:-0}" == "1" && -f "$FAKE_RAILWAY_STATE/apply-attempted" ]]; then
      exit 1
    fi
    if [[ -f "$FAKE_RAILWAY_STATE/exact" ]]; then
      echo '{"exact":true,"pending":[]}'
    else
      echo '{"exact":false,"pending":[{"name":"m9999_test","impact":"maintenance-required"}]}'
    fi
    ;;
  apply)
    touch "$FAKE_RAILWAY_STATE/apply-attempted"
    [[ "${FAKE_FAIL_APPLY:-0}" == "rollback" ]] && exit 1
    touch "$FAKE_RAILWAY_STATE/exact"
    [[ "${FAKE_FAIL_APPLY:-0}" == "committed-error" ]] && exit 1
    echo '{"exact":true,"migration":"applied"}'
    ;;
  verify)
    [[ -f "$FAKE_RAILWAY_STATE/exact" ]]
    echo '{"exact":true}'
    ;;
  *)
    exit 2
    ;;
esac
FAKE
chmod +x "$test_dir/maintenance"

cat > "$test_dir/bin/railway" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FAKE_RAILWAY_TRACE"

if [[ "$1" == "status" ]]; then
  cat <<'JSON'
{"id":"project-test","environments":{"edges":[{"node":{"id":"production","name":"production"}}]},"services":{"edges":[{"node":{"id":"scope-api","name":"scope-api"}},{"node":{"id":"scope-worker","name":"scope-worker"}},{"node":{"id":"scope-postgres","name":"scope-postgres"}}]}}
JSON
  exit 0
fi

if [[ "$1" == "run" ]]; then
  service=""
  while [[ "$1" != "--" ]]; do
    if [[ "$1" == "--service" ]]; then
      service="$2"
      shift 2
    else
      shift
    fi
  done
  [[ "$service" == "scope-postgres" ]]
  shift
  DATABASE_PUBLIC_URL="postgres://public-database.test/scope" "$@"
  exit $?
fi

if [[ "$1 $2" == "deployment list" ]]; then
  service=""
  while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == "--service" ]]; then service="$2"; shift 2; else shift; fi
  done
  if [[ -f "$FAKE_RAILWAY_STATE/no-history-${service}" && ! -f "$FAKE_RAILWAY_STATE/up-${service}" ]]; then
    echo '[]'
    exit 0
  fi
  id="old-${service}"
  [[ -f "$FAKE_RAILWAY_STATE/up-${service}" ]] && id="new-${service}"
  if [[ -f "$FAKE_RAILWAY_STATE/crashed-${service}" ]]; then
    printf '[{"id":"%s","status":"CRASHED","createdAt":"2026-01-02T00:00:00Z"}]\n' "$id"
    exit 0
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/skipped-${service}" ]]; then
    printf '[{"id":"skip-%s","status":"SKIPPED","createdAt":"2026-01-02T00:00:00Z","meta":{"skippedReason":"identical"}},{"id":"new-%s","status":"SUCCESS","createdAt":"2026-01-01T00:00:00Z"}]\n' "$service" "$service"
    exit 0
  fi
  printf '[{"id":"%s","status":"SUCCESS","createdAt":"2026-01-01T00:00:00Z"}]\n' "$id"
  exit 0
fi

if [[ "$1 $2" == "service list" ]]; then
  api_deployment='"old-scope-api"'
  worker_deployment='"old-scope-worker"'
  api_status=SUCCESS
  worker_status=SUCCESS
  api_replicas='{"configured":1,"running":1,"crashed":0,"exited":0,"total":1}'
  worker_replicas='{"configured":1,"running":1,"crashed":0,"exited":0,"total":1}'
  api_stopped=false
  worker_stopped=false
  if [[ -f "$FAKE_RAILWAY_STATE/stopped-scope-api" ]]; then
    api_stopped=true
    api_deployment=null
    api_replicas='{"configured":1,"running":0,"crashed":0,"exited":0,"total":0}'
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/stopped-scope-worker" ]]; then
    worker_stopped=true
    worker_deployment=null
    worker_replicas='{"configured":1,"running":0,"crashed":0,"exited":0,"total":0}'
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/no-history-scope-api" && ! -f "$FAKE_RAILWAY_STATE/up-scope-api" ]]; then
    api_deployment=null
    api_replicas=null
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/no-history-scope-worker" && ! -f "$FAKE_RAILWAY_STATE/up-scope-worker" ]]; then
    worker_deployment=null
    worker_replicas=null
  fi
  [[ -f "$FAKE_RAILWAY_STATE/up-scope-api" && "$api_stopped" == "false" ]] && api_deployment='"new-scope-api"'
  [[ -f "$FAKE_RAILWAY_STATE/up-scope-worker" && "$worker_stopped" == "false" ]] && worker_deployment='"new-scope-worker"'
  if [[ -f "$FAKE_RAILWAY_STATE/up-scope-api" && "$api_stopped" == "false" && ! -f "$FAKE_RAILWAY_STATE/crashed-scope-api" ]]; then
    api_replicas="{\"configured\":${FAKE_NEW_REPLICAS:-1},\"running\":${FAKE_NEW_REPLICAS:-1},\"crashed\":0,\"exited\":0,\"total\":${FAKE_NEW_REPLICAS:-1}}"
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/up-scope-worker" && "$worker_stopped" == "false" && ! -f "$FAKE_RAILWAY_STATE/crashed-scope-worker" ]]; then
    worker_replicas="{\"configured\":${FAKE_NEW_REPLICAS:-1},\"running\":${FAKE_NEW_REPLICAS:-1},\"crashed\":0,\"exited\":0,\"total\":${FAKE_NEW_REPLICAS:-1}}"
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/crashed-scope-api" && "$api_stopped" == "false" ]]; then
    api_status=CRASHED
    api_replicas='{"configured":1,"running":0,"crashed":1,"exited":0,"total":1}'
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/crashed-scope-worker" && "$worker_stopped" == "false" ]]; then
    worker_status=CRASHED
    worker_replicas='{"configured":1,"running":0,"crashed":1,"exited":0,"total":1}'
  fi
  if [[ "${FAKE_DEGRADED_SERVICE:-}" == "scope-api" && "$api_stopped" == "false" ]]; then
    api_replicas='{"configured":2,"running":1,"crashed":1,"exited":0,"total":2}'
  fi
  if [[ "${FAKE_DEGRADED_SERVICE:-}" == "scope-worker" && "$worker_stopped" == "false" ]]; then
    worker_replicas='{"configured":2,"running":1,"crashed":1,"exited":0,"total":2}'
  fi
  if [[ "${FAKE_DEGRADE_WORKER_AFTER_API_STOP:-0}" == "1" \
    && -f "$FAKE_RAILWAY_STATE/stopped-scope-api" && "$worker_stopped" == "false" ]]; then
    worker_status=CRASHED
    worker_replicas='{"configured":1,"running":0,"crashed":1,"exited":0,"total":1}'
  fi
  printf '[{"id":"scope-api","name":"scope-api","status":"%s","deploymentId":%s,"deploymentStopped":%s,"replicas":%s},{"id":"scope-worker","name":"scope-worker","status":"%s","deploymentId":%s,"deploymentStopped":%s,"replicas":%s}]\n' "$api_status" "$api_deployment" "$api_stopped" "$api_replicas" "$worker_status" "$worker_deployment" "$worker_stopped" "$worker_replicas"
  exit 0
fi

if [[ "$1" == "up" ]]; then
  service=""
  while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == "--service" ]]; then service="$2"; shift 2; else shift; fi
  done
  [[ "${FAKE_FAIL_UP_SERVICE:-}" == "$service" ]] && exit 1
  rm -f "$FAKE_RAILWAY_STATE/stopped-${service}"
  if [[ "${FAKE_CRASH_UP_SERVICE:-}" == "$service" ]]; then
    touch "$FAKE_RAILWAY_STATE/up-${service}" "$FAKE_RAILWAY_STATE/crashed-${service}"
    printf '{"deploymentId":"new-%s"}\n' "$service"
    exit 0
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/up-${service}" ]]; then
    touch "$FAKE_RAILWAY_STATE/skipped-${service}"
    printf '{"deploymentId":"skip-%s"}\n' "$service"
    exit 0
  fi
  touch "$FAKE_RAILWAY_STATE/up-${service}"
  printf '{"deploymentId":"new-%s"}\n' "$service"
  exit 0
fi

echo "unexpected fake Railway invocation: $*" >&2
exit 2
FAKE
chmod +x "$test_dir/bin/railway"

cat > "$test_dir/bin/curl" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
cat >/dev/null
request=""
while [[ "$#" -gt 0 ]]; do
  if [[ "$1" == "--data-binary" ]]; then request="$2"; shift 2; else shift; fi
done
case "$request" in
  *scope-api*) service="scope-api" ;;
  *scope-worker*) service="scope-worker" ;;
  *) exit 2 ;;
esac
case "$request" in
  *deploymentRedeploy*) mutation=deploymentRedeploy ;;
  *deploymentRemove*) mutation=deploymentRemove ;;
  *deploymentStop*) mutation=deploymentStop ;;
  *) exit 2 ;;
esac
printf 'graphql %s %s\n' "$mutation" "$service" >> "$FAKE_RAILWAY_TRACE"
[[ "${FAKE_DENY_REMOVE_SERVICE:-}" == "$service" ]] && exit 1
if [[ "$mutation" == "deploymentRedeploy" ]]; then
  rm -f "$FAKE_RAILWAY_STATE/stopped-${service}" "$FAKE_RAILWAY_STATE/crashed-${service}"
else
  touch "$FAKE_RAILWAY_STATE/stopped-${service}"
fi
if [[ "$mutation" == "deploymentRedeploy" ]]; then
  printf '{"data":{"deploymentRedeploy":{"id":"restored-%s"}}}\n' "$service"
else
  printf '{"data":{"%s":true}}\n' "$mutation"
fi
FAKE
chmod +x "$test_dir/bin/curl"

run_cutover() {
  local name="$1"
  local fail_apply="$2"
  local initial_exact="${3:-0}"
  local fail_up_service="${4:-}"
  local fail_recovery_plan="${5:-0}"
  local recover_closed_cutover="${6:-0}"
  local initial_closed="${7:-0}"
  local no_history="${8:-0}"
  local fail_redeploy_service="${9:-}"
  local fail_first_plan="${10:-0}"
  local deny_remove_service="${11:-}"
  local crash_up_service="${12:-}"
  local new_replicas="${13:-1}"
  local degraded_service="${14:-}"
  local degrade_worker_after_api_stop="${15:-0}"
  local state="$test_dir/$name-state"
  local trace="$test_dir/$name-trace"
  mkdir -p "$state"
  [[ "$initial_exact" == "1" ]] && touch "$state/exact"
  if [[ "$initial_closed" == "1" ]]; then
    touch "$state/stopped-scope-api" "$state/stopped-scope-worker"
  fi
  if [[ "$no_history" == "1" ]]; then
    touch "$state/no-history-scope-api" "$state/no-history-scope-worker"
  fi
  : > "$trace"
  set +e
  PATH="$test_dir/bin:$PATH" \
    FAKE_RAILWAY_STATE="$state" \
    FAKE_RAILWAY_TRACE="$trace" \
    FAKE_FAIL_APPLY="$fail_apply" \
    FAKE_FAIL_UP_SERVICE="$fail_up_service" \
    FAKE_CRASH_UP_SERVICE="$crash_up_service" \
    FAKE_NEW_REPLICAS="$new_replicas" \
    FAKE_DEGRADED_SERVICE="$degraded_service" \
    FAKE_DEGRADE_WORKER_AFTER_API_STOP="$degrade_worker_after_api_stop" \
    FAKE_FAIL_RECOVERY_PLAN="$fail_recovery_plan" \
    FAKE_FAIL_REDEPLOY_SERVICE="$fail_redeploy_service" \
    FAKE_FAIL_FIRST_PLAN="$fail_first_plan" \
    FAKE_DENY_REMOVE_SERVICE="$deny_remove_service" \
    RAILWAY_PROJECT_ID="project-test" \
    RAILWAY_TOKEN="token-test" \
    SCOPE_RAILWAY_ENVIRONMENT_ID="production" \
    SCOPE_RAILWAY_API_SERVICE_ID="scope-api" \
    SCOPE_RAILWAY_WORKER_SERVICE_ID="scope-worker" \
    SCOPE_RAILWAY_DATABASE_SERVICE_ID="scope-postgres" \
    SCOPE_RAILWAY_API_REPLICAS="1" \
    SCOPE_RAILWAY_WORKER_REPLICAS="1" \
    SCOPE_MAINTENANCE_BINARY="$test_dir/maintenance" \
    SCOPE_RECOVER_CLOSED_CUTOVER="$recover_closed_cutover" \
    bash "$root/.github/scripts/deploy-backend-railway.sh" "$test_dir/api" "$test_dir/worker"
  result=$?
  set -e
  printf '%s\n' "$result" > "$test_dir/$name-result"
}

assert_in_order() {
  local trace="$1"
  shift
  local previous=0 pattern line
  for pattern in "$@"; do
    line="$(
      grep -n -F "$pattern" "$trace" \
        | cut -d: -f1 \
        | awk -v previous="$previous" '$1 > previous { print; exit }'
    )"
    [[ -n "$line" && "$line" -gt "$previous" ]] || {
      echo "missing or out-of-order '$pattern' in $trace" >&2
      return 1
    }
    previous="$line"
  done
}

run_cutover success 0
[[ "$(cat "$test_dir/success-result")" == "0" ]]
assert_in_order "$test_dir/success-trace" \
  "$test_dir/maintenance plan" \
  "graphql deploymentStop scope-api" \
  "graphql deploymentStop scope-worker" \
  "$test_dir/maintenance apply" \
  "$test_dir/maintenance verify" \
  "up $test_dir/worker" \
  "up $test_dir/api"

run_cutover changed-replicas 0 0 "" 0 0 0 0 "" 0 "" "" 2
[[ "$(cat "$test_dir/changed-replicas-result")" == "0" ]]

run_cutover degraded 0 0 "" 0 0 0 0 "" 0 "" "" 1 scope-worker
[[ "$(cat "$test_dir/degraded-result")" != "0" ]]
if grep -F "graphql deployment" "$test_dir/degraded-trace" \
  || grep -F "$test_dir/maintenance apply" "$test_dir/degraded-trace"; then
  echo "maintenance must not start from a degraded service" >&2
  exit 1
fi

run_cutover degraded-during-shutdown 0 0 "" 0 0 0 0 "" 0 "" "" 1 "" 1
[[ "$(cat "$test_dir/degraded-during-shutdown-result")" != "0" ]]
assert_in_order "$test_dir/degraded-during-shutdown-trace" \
  "graphql deploymentStop scope-api" \
  "graphql deploymentRedeploy scope-api"
if grep -F "graphql deploymentStop scope-worker" "$test_dir/degraded-during-shutdown-trace" \
  || grep -F "graphql deploymentRemove scope-worker" "$test_dir/degraded-during-shutdown-trace"; then
  echo "a service that degrades before shutdown must remain recoverable" >&2
  exit 1
fi

run_cutover crashed-worker 0 0 "" 0 0 0 0 "" 0 "" scope-worker
[[ "$(cat "$test_dir/crashed-worker-result")" != "0" ]]
assert_in_order "$test_dir/crashed-worker-trace" \
  "graphql deploymentStop scope-worker" \
  "graphql deploymentRemove scope-worker"

run_cutover rollback rollback
[[ "$(cat "$test_dir/rollback-result")" != "0" ]]
assert_in_order "$test_dir/rollback-trace" \
  "$test_dir/maintenance apply" \
  "$test_dir/maintenance plan" \
  "graphql deploymentRedeploy scope-worker" \
  "graphql deploymentRedeploy scope-api"
if grep -F "up $test_dir/api" "$test_dir/rollback-trace"; then
  echo "failed migration must not deploy the new API" >&2
  exit 1
fi

run_cutover unknown committed-error 0 "" 1
[[ "$(cat "$test_dir/unknown-result")" != "0" ]]
if grep -F "deploymentRedeploy" "$test_dir/unknown-trace"; then
  echo "unknown migration state must not restore old deployments" >&2
  exit 1
fi
if grep -F "up $test_dir/api" "$test_dir/unknown-trace"; then
  echo "unknown migration state must not deploy new binaries" >&2
  exit 1
fi

run_cutover rolling 0 1
[[ "$(cat "$test_dir/rolling-result")" == "0" ]]
assert_in_order "$test_dir/rolling-trace" \
  "$test_dir/maintenance plan" \
  "up $test_dir/api" \
  "$test_dir/maintenance verify" \
  "up $test_dir/worker"
if grep -F "graphql deployment" "$test_dir/rolling-trace"; then
  echo "exact-schema deployment must stay on the rolling path" >&2
  exit 1
fi

run_cutover transient-plan 0 1 "" 0 0 0 0 "" 1
[[ "$(cat "$test_dir/transient-plan-result")" == "0" ]]
[[ "$(grep -F -x -c "$test_dir/maintenance plan" "$test_dir/transient-plan-trace")" == "2" ]]

run_cutover interrupted 0 0 scope-worker
[[ "$(cat "$test_dir/interrupted-result")" != "0" ]]
run_cutover interrupted 0 0 "" 0 1
[[ "$(cat "$test_dir/interrupted-result")" == "0" ]]
assert_in_order "$test_dir/interrupted-trace" \
  "$test_dir/maintenance plan" \
  "$test_dir/maintenance verify" \
  "up $test_dir/worker" \
  "up $test_dir/api"
if grep -F "$test_dir/maintenance apply" "$test_dir/interrupted-trace"; then
  echo "post-commit recovery must not reapply migrations" >&2
  exit 1
fi

run_cutover intentionally-closed 0 1 "" 0 0 1
[[ "$(cat "$test_dir/intentionally-closed-result")" != "0" ]]
if grep -F "up $test_dir/api" "$test_dir/intentionally-closed-trace"; then
  echo "an ordinary deployment must not reopen intentionally closed writers" >&2
  exit 1
fi

run_cutover bootstrap 0 1 "" 0 0 1 1
[[ "$(cat "$test_dir/bootstrap-result")" == "0" ]]
assert_in_order "$test_dir/bootstrap-trace" \
  "$test_dir/maintenance plan" \
  "$test_dir/maintenance verify" \
  "up $test_dir/worker" \
  "up $test_dir/api"

run_cutover partial-reopen 0 0 scope-api
[[ "$(cat "$test_dir/partial-reopen-result")" != "0" ]]
assert_in_order "$test_dir/partial-reopen-trace" \
  "graphql deploymentStop scope-api" \
  "graphql deploymentStop scope-worker" \
  "up $test_dir/worker" \
  "up $test_dir/api" \
  "graphql deploymentStop scope-worker"
[[ "$(grep -F -c "graphql deploymentStop scope-api" "$test_dir/partial-reopen-trace")" == "1" ]]

run_cutover denied-api 0 0 "" 0 0 0 0 "" 0 scope-api
[[ "$(cat "$test_dir/denied-api-result")" != "0" ]]
[[ "$(grep -F -c "graphql deploymentStop" "$test_dir/denied-api-trace")" == "1" ]]
if grep -F "$test_dir/maintenance apply" "$test_dir/denied-api-trace"; then
  echo "a denied API shutdown must fail before migration without attempting rollback mutations" >&2
  exit 1
fi

run_cutover denied-worker 0 0 "" 0 0 0 0 "" 0 scope-worker
[[ "$(cat "$test_dir/denied-worker-result")" != "0" ]]
assert_in_order "$test_dir/denied-worker-trace" \
  "graphql deploymentStop scope-api" \
  "graphql deploymentStop scope-worker" \
  "graphql deploymentRedeploy scope-api"
if grep -F "graphql deploymentRedeploy scope-worker" "$test_dir/denied-worker-trace"; then
  echo "a worker shutdown denial must not restore a worker that was never closed" >&2
  exit 1
fi

echo "backend deployment cutover tests passed"
