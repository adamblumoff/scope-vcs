#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT
mkdir -p "$test_dir/bin" "$test_dir/api" "$test_dir/worker"
touch "$test_dir/maintenance"
chmod +x "$test_dir/maintenance"

cat > "$test_dir/bin/railway" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FAKE_RAILWAY_TRACE"

if [[ "$1 $2" == "run --project" ]]; then
  command="${!#}"
  case "$command" in
    plan)
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
  esac
  exit 0
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
  if [[ -f "$FAKE_RAILWAY_STATE/skipped-${service}" ]]; then
    printf '[{"id":"skip-%s","status":"SKIPPED","createdAt":"2026-01-02T00:00:00Z","meta":{"skippedReason":"identical"}},{"id":"new-%s","status":"SUCCESS","createdAt":"2026-01-01T00:00:00Z","meta":{"serviceManifest":{"deploy":{"region":"us-west","numReplicas":1}}}}]\n' "$service" "$service"
    exit 0
  fi
  printf '[{"id":"%s","status":"SUCCESS","createdAt":"2026-01-01T00:00:00Z","meta":{"serviceManifest":{"deploy":{"region":"us-west","numReplicas":1}}}}]\n' "$id"
  exit 0
fi

if [[ "$1 $2" == "service list" ]]; then
  api=1
  worker=1
  [[ -f "$FAKE_RAILWAY_STATE/stopped-scope-api" ]] && api=0
  [[ -f "$FAKE_RAILWAY_STATE/stopped-scope-worker" ]] && worker=0
  printf '[{"id":"api","name":"scope-api","status":"SUCCESS","replicas":{"running":%s,"crashed":0,"exited":0,"total":%s}},{"id":"worker","name":"scope-worker","status":"SUCCESS","replicas":{"running":%s,"crashed":0,"exited":0,"total":%s}}]\n' "$api" "$api" "$worker" "$worker"
  exit 0
fi

if [[ "$1 $2" == "service scale" ]]; then
  service=""
  stopped=0
  for argument in "$@"; do
    [[ "$argument" == "scope-api" || "$argument" == "scope-worker" ]] && service="$argument"
    [[ "$argument" == *=0 ]] && stopped=1
  done
  if [[ "$stopped" == "0" && "${FAKE_FAIL_SCALE_SERVICE:-}" == "$service" ]]; then
    exit 1
  fi
  if [[ "$stopped" == "1" ]]; then
    touch "$FAKE_RAILWAY_STATE/stopped-${service}"
  else
    rm -f "$FAKE_RAILWAY_STATE/stopped-${service}"
  fi
  echo '{"regions":{"us-west":{"numReplicas":1}}}'
  exit 0
fi

if [[ "$1" == "up" ]]; then
  service=""
  while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == "--service" ]]; then service="$2"; shift 2; else shift; fi
  done
  [[ "${FAKE_FAIL_UP_SERVICE:-}" == "$service" ]] && exit 1
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

run_cutover() {
  local name="$1"
  local fail_apply="$2"
  local initial_exact="${3:-0}"
  local fail_up_service="${4:-}"
  local fail_recovery_plan="${5:-0}"
  local recover_closed_cutover="${6:-0}"
  local initial_closed="${7:-0}"
  local no_history="${8:-0}"
  local fail_scale_service="${9:-}"
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
    FAKE_FAIL_RECOVERY_PLAN="$fail_recovery_plan" \
    FAKE_FAIL_SCALE_SERVICE="$fail_scale_service" \
    RAILWAY_PROJECT_ID="project-test" \
    RAILWAY_TOKEN="token-test" \
    SCOPE_MAINTENANCE_BINARY="$test_dir/maintenance" \
    SCOPE_API_TOPOLOGY="us-west=1" \
    SCOPE_WORKER_TOPOLOGY="us-west=1" \
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
  "service scale --project project-test --environment production --service scope-api --json us-west=0" \
  "service scale --project project-test --environment production --service scope-worker --json us-west=0" \
  "$test_dir/maintenance apply" \
  "up $test_dir/api" \
  "up $test_dir/worker" \
  "$test_dir/maintenance verify" \
  "service scale --project project-test --environment production --service scope-worker --json us-west=1" \
  "service scale --project project-test --environment production --service scope-api --json us-west=1"

run_cutover rollback rollback
[[ "$(cat "$test_dir/rollback-result")" != "0" ]]
assert_in_order "$test_dir/rollback-trace" \
  "$test_dir/maintenance apply" \
  "$test_dir/maintenance plan" \
  "service scale --project project-test --environment production --service scope-worker --json us-west=1" \
  "service scale --project project-test --environment production --service scope-api --json us-west=1"
if grep -F "up $test_dir/api" "$test_dir/rollback-trace"; then
  echo "failed migration must not deploy the new API" >&2
  exit 1
fi

run_cutover unknown committed-error 0 "" 1
[[ "$(cat "$test_dir/unknown-result")" != "0" ]]
if grep -F "service scale --project project-test --environment production --service scope-worker --json us-west=1" "$test_dir/unknown-trace" \
  || grep -F "service scale --project project-test --environment production --service scope-api --json us-west=1" "$test_dir/unknown-trace"; then
  echo "unknown migration state must keep both writers closed" >&2
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
if grep -F "service scale" "$test_dir/rolling-trace"; then
  echo "exact-schema deployment must stay on the rolling path" >&2
  exit 1
fi

run_cutover interrupted 0 0 scope-worker
[[ "$(cat "$test_dir/interrupted-result")" != "0" ]]
run_cutover interrupted 0 0 "" 0 1
[[ "$(cat "$test_dir/interrupted-result")" == "0" ]]
assert_in_order "$test_dir/interrupted-trace" \
  "$test_dir/maintenance plan" \
  "up $test_dir/api" \
  "up $test_dir/worker" \
  "$test_dir/maintenance verify" \
  "service scale --project project-test --environment production --service scope-worker --json us-west=1" \
  "service scale --project project-test --environment production --service scope-api --json us-west=1"
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
  "up $test_dir/api" \
  "up $test_dir/worker" \
  "service scale --project project-test --environment production --service scope-worker --json us-west=1" \
  "service scale --project project-test --environment production --service scope-api --json us-west=1"

run_cutover partial-reopen 0 0 "" 0 0 0 0 scope-api
[[ "$(cat "$test_dir/partial-reopen-result")" != "0" ]]
assert_in_order "$test_dir/partial-reopen-trace" \
  "service scale --project project-test --environment production --service scope-worker --json us-west=1" \
  "service scale --project project-test --environment production --service scope-api --json us-west=1" \
  "service scale --project project-test --environment production --service scope-api --json us-west=0" \
  "service scale --project project-test --environment production --service scope-worker --json us-west=0"

echo "backend deployment cutover tests passed"
