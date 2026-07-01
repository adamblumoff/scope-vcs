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

ensure_service_exists() {
  local service_name="$1"
  local services_json

  services_json="$(
    railway service list \
      --project "$RAILWAY_PROJECT_ID" \
      --environment production \
      --json
  )"

  if ! SERVICES_JSON="$services_json" SERVICE_NAME="$service_name" node -e 'const services = JSON.parse(process.env.SERVICES_JSON || "[]"); const name = process.env.SERVICE_NAME || ""; process.exit(services.some((service) => service.name === name || service.id === name) ? 0 : 1);'; then
    echo "Railway service '${service_name}' was not found in the production environment."
    echo "Create the service in Railway, configure its variables, then rerun this workflow."
    return 1
  fi
}

print_deployment_logs() {
  local service_name="$1"
  local deployment_id="$2"

  echo "::group::Railway build logs for ${service_name}/${deployment_id}"
  railway logs "$deployment_id" \
    --project "$RAILWAY_PROJECT_ID" \
    --service "$service_name" \
    --environment production \
    --build \
    --lines 200 || true
  echo "::endgroup::"

  echo "::group::Railway deploy logs for ${service_name}/${deployment_id}"
  railway logs "$deployment_id" \
    --project "$RAILWAY_PROJECT_ID" \
    --service "$service_name" \
    --environment production \
    --deployment \
    --lines 200 || true
  echo "::endgroup::"
}

service_health_line() {
  local service_name="$1"
  local services_json

  services_json="$(
    railway service list \
      --project "$RAILWAY_PROJECT_ID" \
      --environment production \
      --json
  )"

  SERVICES_JSON="$services_json" SERVICE_NAME="$service_name" node -e '
const services = JSON.parse(process.env.SERVICES_JSON || "[]");
const name = process.env.SERVICE_NAME || "";
const service = services.find((candidate) => candidate.name === name || candidate.id === name);
if (!service) process.exit(1);
const replicas = service.replicas || {};
console.log([service.status || "", replicas.running || 0, replicas.crashed || 0, replicas.exited || 0, replicas.total || 0].join("\t"));
'
}

wait_for_deployment() {
  local service_name="$1"
  local deployment_id="$2"
  local deadline=$((SECONDS + 900))
  local deployment_json
  local deployment_line
  local deployment_status
  local skipped_reason
  local health_line
  local service_status
  local running_replicas
  local crashed_replicas
  local exited_replicas
  local total_replicas

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
        DEPLOYMENT_ID="$deployment_id" \
        node -e 'const deployments = JSON.parse(process.env.DEPLOYMENTS_JSON || "[]"); const id = process.env.DEPLOYMENT_ID || ""; const deployment = deployments.find((candidate) => candidate.id === id); if (deployment) console.log([deployment.id, deployment.status, deployment.meta?.skippedReason || ""].join("\t"));'
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
            health_line="$(service_health_line "$service_name" || true)"
            if [ -n "$health_line" ]; then
              IFS=$'\t' read -r service_status running_replicas crashed_replicas exited_replicas total_replicas <<< "$health_line"
              if [ "$service_status" = "SUCCESS" ] && [ "${running_replicas:-0}" -gt 0 ]; then
                return 0
              fi
            fi
            echo "Railway skipped deployment, but ${service_name} is not currently healthy."
            echo "Service status: ${service_status:-unknown}; replicas running=${running_replicas:-0}, crashed=${crashed_replicas:-0}, exited=${exited_replicas:-0}, total=${total_replicas:-0}."
            return 1
            ;;
          FAILED|CRASHED|REMOVED)
            print_deployment_logs "$service_name" "$deployment_id"
            return 1
            ;;
        esac
      else
        echo "Waiting for Railway deployment $deployment_id to appear..."
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
deploy_output=""
deployment_id=""

ensure_service_exists "$service_name"

deploy_output="$(
  railway up "$upload_root" \
  --path-as-root \
  --no-gitignore \
  --project "$RAILWAY_PROJECT_ID" \
  --service "$service_name" \
  --environment production \
  --message "$deploy_message" \
  --detach \
  --json
)"
printf '%s\n' "$deploy_output"

deployment_id="$(
  DEPLOY_OUTPUT="$deploy_output" node -e '
const lines = (process.env.DEPLOY_OUTPUT || "").split(/\r?\n/).filter(Boolean);
for (const line of lines) {
  try {
    const parsed = JSON.parse(line);
    if (parsed && typeof parsed.deploymentId === "string" && parsed.deploymentId.length > 0) {
      process.stdout.write(parsed.deploymentId);
      process.exit(0);
    }
  } catch {}
}
process.exit(1);
'
)"

wait_for_deployment "$service_name" "$deployment_id"
