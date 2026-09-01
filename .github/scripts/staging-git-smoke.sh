#!/usr/bin/env bash
set -euo pipefail

: "${SCOPE_API_URL:?SCOPE_API_URL is required}"
: "${SCOPE_GIT_ROUTER_URL:?SCOPE_GIT_ROUTER_URL is required}"
: "${SCOPE_CLI_BINARY:?SCOPE_CLI_BINARY is required}"
: "${SCOPE_EXCHANGE_TOKEN_PATH:?SCOPE_EXCHANGE_TOKEN_PATH is required}"
: "${SCOPE_GIT_SMOKE_DIR:?SCOPE_GIT_SMOKE_DIR is required}"

if [[ ! "$SCOPE_API_URL" =~ ^https://[A-Za-z0-9.-]+(:[0-9]+)?$ ]]; then
  echo "SCOPE_API_URL must be an HTTPS origin without a path." >&2
  exit 2
fi
if [[ ! "$SCOPE_GIT_ROUTER_URL" =~ ^https://[A-Za-z0-9.-]+(:[0-9]+)?$ ]]; then
  echo "SCOPE_GIT_ROUTER_URL must be an HTTPS origin without a path." >&2
  exit 2
fi
if [[ ! -x "$SCOPE_CLI_BINARY" ]]; then
  echo "SCOPE_CLI_BINARY must be executable." >&2
  exit 2
fi
if [[ ! -f "$SCOPE_EXCHANGE_TOKEN_PATH" ]]; then
  echo "The staging exchange token file is missing." >&2
  exit 2
fi
if [[ "$(stat -c '%a' "$SCOPE_EXCHANGE_TOKEN_PATH")" != "600" ]]; then
  echo "The staging exchange token file must have mode 0600." >&2
  exit 2
fi
if [[ "$SCOPE_GIT_SMOKE_DIR" != /* || "$SCOPE_GIT_SMOKE_DIR" == "/" ]]; then
  echo "SCOPE_GIT_SMOKE_DIR must be a specific absolute directory." >&2
  exit 2
fi
if [[ ! -d "$SCOPE_GIT_SMOKE_DIR" || -L "$SCOPE_GIT_SMOKE_DIR" ]]; then
  echo "SCOPE_GIT_SMOKE_DIR must be an existing physical directory." >&2
  exit 2
fi

smoke_dir="$(realpath "$SCOPE_GIT_SMOKE_DIR")"
token_path="$SCOPE_EXCHANGE_TOKEN_PATH"
if [[ "$token_path" != "$smoke_dir/exchange-token" ]]; then
  echo "The staging exchange token must be inside SCOPE_GIT_SMOKE_DIR." >&2
  exit 2
fi
cleanup() {
  rm -f -- "$token_path"
  rm -rf -- "$smoke_dir"
}
trap cleanup EXIT

umask 077
mkdir -p "$smoke_dir/config"
chmod 0700 "$smoke_dir" "$smoke_dir/config"
export XDG_CONFIG_HOME="$smoke_dir/config"
cli_binary="$(realpath "$SCOPE_CLI_BINARY")"
cli_directory="$(dirname "$cli_binary")"
export PATH="$cli_directory:$PATH"

repo="dev/update-demo"
public_url="$SCOPE_GIT_ROUTER_URL/git/public/$repo"
permissioned_url="$SCOPE_GIT_ROUTER_URL/git/permissioned/$repo"
public_checkout="$smoke_dir/public"
permissioned_checkout="$smoke_dir/permissioned"

discovery_headers="$smoke_dir/router-headers"
discovery_body="$smoke_dir/router-body"
status="$(curl --silent --show-error --max-redirs 0 \
  --output "$discovery_body" \
  --dump-header "$discovery_headers" \
  --write-out '%{http_code}' \
  "$public_url/info/refs?service=git-upload-pack")"
if [[ "$status" != "200" ]] || ! grep -Eiq '^x-scope-git-router:[[:space:]]*1[[:space:]]*$' "$discovery_headers"; then
  echo "The staging router domain did not serve Git discovery directly." >&2
  exit 1
fi
rm -f -- "$discovery_headers" "$discovery_body"

GIT_TERMINAL_PROMPT=0 git -c credential.helper= clone --quiet "$public_url" "$public_checkout"
test "$(git -C "$public_checkout" remote get-url origin)" = "$public_url"
test -f "$public_checkout/README.md"
test ! -e "$public_checkout/internal/notes.md"

SCOPE_API_URL="$SCOPE_API_URL" "$cli_binary" login --exchange-file "$token_path"
rm -f -- "$token_path"
SCOPE_API_URL="$SCOPE_API_URL" "$cli_binary" clone "$repo" "$permissioned_checkout"
test "$(git -C "$permissioned_checkout" remote get-url origin)" = "$permissioned_url"
test -f "$permissioned_checkout/internal/notes.md"
GIT_TERMINAL_PROMPT=0 git -C "$permissioned_checkout" fetch --quiet --prune origin

marker="Scope staging router smoke ${GITHUB_SHA:-manual}"
printf '\n%s\n' "$marker" >> "$permissioned_checkout/README.md"
git -C "$permissioned_checkout" add README.md
git -C "$permissioned_checkout" \
  -c user.name='Scope staging smoke' \
  -c user.email='smoke@example.test' \
  commit --quiet -m 'Exercise the staging Git router'
expected_head="$(git -C "$permissioned_checkout" rev-parse HEAD)"
(
  cd "$permissioned_checkout"
  SCOPE_API_URL="$SCOPE_API_URL" "$cli_binary" push --no-review --remote origin
)

GIT_TERMINAL_PROMPT=0 git -C "$public_checkout" -c credential.helper= fetch --quiet origin main
actual_head="$(git -C "$public_checkout" rev-parse FETCH_HEAD)"
if [[ "$actual_head" != "$expected_head" ]]; then
  echo "The public projection did not advance to the accepted staging push." >&2
  exit 1
fi
git -C "$public_checkout" show FETCH_HEAD:README.md | grep -Fqx "$marker"
echo "Staging Git router smoke passed for dev/update-demo."
