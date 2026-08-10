#!/usr/bin/env bash
set -euo pipefail

if [ -z "${RAILWAY_TOKEN:-}" ]; then
  echo "Set the RAILWAY_TOKEN repository secret before verifying the runner cutover."
  exit 1
fi

if [ -z "${RAILWAY_PROJECT_ID:-}" ]; then
  echo "Set the RAILWAY_PROJECT_ID repository secret before verifying the runner cutover."
  exit 1
fi

cutover_json="$(
  # Railway injects these variables for the child shell.
  # shellcheck disable=SC2016
  railway run \
    --project "$RAILWAY_PROJECT_ID" \
    --service scope-api \
    --environment production \
    bash -lc 'curl --silent --show-error --fail-with-body --max-time 20 --header "Authorization: Bearer $SCOPE_OPERATOR_TOKEN" "https://$RAILWAY_PUBLIC_DOMAIN/v1/admin/runner-cutover"'
)"

cutover_status="$(
  # shellcheck disable=SC2016
  CUTOVER_JSON="$cutover_json" node -e '
const cutover = JSON.parse(process.env.CUTOVER_JSON || "{}");
if (typeof cutover.state !== "string" || !Number.isSafeInteger(cutover.enabled_runner_count)) process.exit(1);
process.stdout.write(`${cutover.state}\t${cutover.enabled_runner_count}`);
'
)"
IFS=$'\t' read -r cutover_state enabled_runner_count <<< "$cutover_status"

if [ "$cutover_state" != "v7-open" ]; then
  echo "Production runner protocol cutover is ${cutover_state:-unknown}."
  echo "On the runner host, run: scope runner cutover --name <runner-name> --repo <owner/repo>"
  exit 1
fi

if [ "$enabled_runner_count" -lt 1 ]; then
  echo "Production has no enabled current-protocol runner."
  echo "On the runner host, run: scope runner cutover --name <runner-name> --repo <owner/repo>"
  exit 1
fi

echo "Production runner protocol cutover is v7-open with ${enabled_runner_count} enabled runner(s)."
