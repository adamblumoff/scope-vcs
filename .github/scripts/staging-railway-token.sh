#!/usr/bin/env bash
set -euo pipefail

action="${1:?usage: staging-railway-token.sh <create|delete>}"
project_id="${SCOPE_RAILWAY_PROJECT_ID:?SCOPE_RAILWAY_PROJECT_ID is required}"
environment_id="${SCOPE_RAILWAY_STAGING_ENVIRONMENT_ID:?SCOPE_RAILWAY_STAGING_ENVIRONMENT_ID is required}"
token_name="${SCOPE_RAILWAY_PROJECT_TOKEN_NAME:?SCOPE_RAILWAY_PROJECT_TOKEN_NAME is required}"
api_token="${RAILWAY_API_TOKEN:?RAILWAY_API_TOKEN is required}"
endpoint="https://backboard.railway.com/graphql/v2"

graphql() {
  local query="$1"
  local variables="$2"
  local request response
  request="$(jq -cn --arg query "$query" --argjson variables "$variables" '{query: $query, variables: $variables}')"
  response="$(
    curl --silent --show-error --fail-with-body \
      --request POST \
      --url "$endpoint" \
      --header "Authorization: Bearer $api_token" \
      --header 'Content-Type: application/json' \
      --data-binary "$request"
  )"
  if jq -e '.errors | length > 0' >/dev/null 2>&1 <<< "$response"; then
    jq -r '.errors[]?.message // "Railway GraphQL request failed"' <<< "$response" >&2
    return 1
  fi
  printf '%s\n' "$response"
}

list_token_ids() {
  local response
  response="$(
    # GraphQL variables are intentionally literal here.
    # shellcheck disable=SC2016
    graphql \
      'query ProjectTokens($projectId: String!) { projectTokens(projectId: $projectId) { edges { node { id name } } } }' \
      "$(jq -cn --arg projectId "$project_id" '{projectId: $projectId}')"
  )"
  jq -r --arg name "$token_name" \
    '.data.projectTokens.edges[]?.node | select(.name == $name) | .id' \
    <<< "$response"
}

case "$action" in
  create)
    if [[ -n "$(list_token_ids)" ]]; then
      echo "A Railway project token already uses this staging run name." >&2
      exit 1
    fi
    create_response="$(
      # GraphQL variables are intentionally literal here.
      # shellcheck disable=SC2016
      graphql \
        'mutation ProjectTokenCreate($input: ProjectTokenCreateInput!) { projectTokenCreate(input: $input) }' \
        "$(jq -cn \
          --arg projectId "$project_id" \
          --arg environmentId "$environment_id" \
          --arg name "$token_name" \
          '{input: {projectId: $projectId, environmentId: $environmentId, name: $name}}')"
    )"
    project_token="$(jq -er '.data.projectTokenCreate | strings | select(length > 0)' <<< "$create_response")"
    token_ids="$(list_token_ids)"
    if [[ "$(wc -l <<< "$token_ids")" -ne 1 || -z "$token_ids" ]]; then
      echo "Railway did not return one project token for this staging run." >&2
      exit 1
    fi
    if [[ -z "${GITHUB_ENV:-}" ]]; then
      echo "GITHUB_ENV is required when creating the staging token." >&2
      exit 1
    fi
    printf '::add-mask::%s\n' "$project_token"
    printf 'RAILWAY_TOKEN=%s\n' "$project_token" >> "$GITHUB_ENV"
    ;;
  delete)
    token_ids="$(list_token_ids)"
    if [[ -z "$token_ids" ]]; then
      exit 0
    fi
    if [[ "$(wc -l <<< "$token_ids")" -ne 1 ]]; then
      echo "More than one Railway project token matches this staging run name." >&2
      exit 1
    fi
    # GraphQL variables are intentionally literal here.
    # shellcheck disable=SC2016
    graphql \
      'mutation ProjectTokenDelete($id: String!) { projectTokenDelete(id: $id) }' \
      "$(jq -cn --arg id "$token_ids" '{id: $id}')" >/dev/null
    if [[ -n "$(list_token_ids)" ]]; then
      echo "Railway project token still exists after deletion." >&2
      exit 1
    fi
    ;;
  *)
    echo "usage: staging-railway-token.sh <create|delete>" >&2
    exit 2
    ;;
esac
