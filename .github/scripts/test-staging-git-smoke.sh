#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
test_root="$(mktemp -d "${TMPDIR:-/tmp}/scope-staging-git-smoke-test.XXXXXX")"
trap 'rm -rf -- "$test_root"' EXIT
fake_bin="$test_root/bin"
smoke_dir="$test_root/smoke"
token_path="$smoke_dir/exchange-token"
trace_path="$test_root/trace"
mkdir -m 0700 "$fake_bin" "$smoke_dir"
printf '%s\n' 'scope_otc_do_not_log_this_value' > "$token_path"
chmod 0600 "$token_path"

cat > "$fake_bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output=''
headers=''
while (($#)); do
  case "$1" in
    --output) output="$2"; shift 2 ;;
    --dump-header) headers="$2"; shift 2 ;;
    --write-out) shift 2 ;;
    *) shift ;;
  esac
done
if [[ "${FAKE_ROUTER_DIRECT:-1}" = '1' ]]; then
  printf 'x-scope-git-router: 1\r\n\r\n' > "$headers"
  status=200
else
  printf 'location: https://api-staging.example.test/git/public/dev/update-demo/info/refs\r\n\r\n' > "$headers"
  status=302
fi
printf 'git discovery' > "$output"
printf 'curl-router\n' >> "$TRACE_PATH"
printf '%s' "$status"
EOF

cat > "$fake_bin/scope" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$1" in
  login)
    test "$2" = '--exchange-file'
    test -s "$3"
    printf 'scope-login-file\n' >> "$TRACE_PATH"
    ;;
  clone)
    destination="$3"
    mkdir -p "$destination/internal"
    printf 'initial\n' > "$destination/README.md"
    printf 'private\n' > "$destination/internal/notes.md"
    printf '%s/git/permissioned/%s\n' "$SCOPE_GIT_ROUTER_URL" "$2" > "$destination/.origin"
    printf 'scope-clone\n' >> "$TRACE_PATH"
    ;;
  push)
    printf 'scope-push\n' >> "$TRACE_PATH"
    ;;
  *) exit 2 ;;
esac
EOF

cat > "$fake_bin/git" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" = '-c' && "$3" = 'clone' ]]; then
  destination="$6"
  mkdir -p "$destination"
  printf 'initial\n' > "$destination/README.md"
  printf '%s\n' "$5" > "$destination/.origin"
  printf 'git-public-clone\n' >> "$TRACE_PATH"
  exit 0
fi
test "$1" = '-C'
directory="$2"
shift 2
case "$1" in
  remote)
    cat "$directory/.origin"
    ;;
  fetch)
    printf 'git-fetch\n' >> "$TRACE_PATH"
    ;;
  add) ;;
  -c) ;;
  rev-parse)
    printf '1111111111111111111111111111111111111111\n'
    ;;
  show)
    printf 'initial\nScope staging router smoke test-sha\n'
    ;;
  *) exit 2 ;;
esac
EOF

chmod 0755 "$fake_bin/curl" "$fake_bin/git" "$fake_bin/scope"
export TRACE_PATH="$trace_path"
export PATH="$fake_bin:$PATH"
output="$test_root/output"
SCOPE_API_URL='https://api-staging.example.test' \
  SCOPE_GIT_ROUTER_URL='https://router-staging.example.test' \
  SCOPE_CLI_BINARY="$fake_bin/scope" \
  SCOPE_EXCHANGE_TOKEN_PATH="$token_path" \
  SCOPE_GIT_SMOKE_DIR="$smoke_dir" \
  GITHUB_SHA='test-sha' \
  bash "$repo_root/.github/scripts/staging-git-smoke.sh" > "$output" 2>&1

test ! -e "$smoke_dir"
test "$(sed -n '1p' "$trace_path")" = 'curl-router'
test "$(sed -n '2p' "$trace_path")" = 'git-public-clone'
test "$(sed -n '3p' "$trace_path")" = 'scope-login-file'
test "$(sed -n '4p' "$trace_path")" = 'scope-clone'
grep -Fxq 'scope-push' "$trace_path"
if grep -Fq 'scope_otc_do_not_log_this_value' "$output" "$trace_path"; then
  echo "staging Git smoke exposed its exchange token" >&2
  exit 1
fi

redirect_dir="$test_root/redirect-smoke"
redirect_token="$redirect_dir/exchange-token"
mkdir -m 0700 "$redirect_dir"
printf '%s\n' 'scope_otc_second_private_value' > "$redirect_token"
chmod 0600 "$redirect_token"
if FAKE_ROUTER_DIRECT=0 \
  SCOPE_API_URL='https://api-staging.example.test' \
  SCOPE_GIT_ROUTER_URL='https://router-staging.example.test' \
  SCOPE_CLI_BINARY="$fake_bin/scope" \
  SCOPE_EXCHANGE_TOKEN_PATH="$redirect_token" \
  SCOPE_GIT_SMOKE_DIR="$redirect_dir" \
  bash "$repo_root/.github/scripts/staging-git-smoke.sh" > "$test_root/redirect-output" 2>&1; then
  echo "staging Git smoke accepted a router redirect" >&2
  exit 1
fi
grep -Fq 'did not serve Git discovery directly' "$test_root/redirect-output"
test ! -e "$redirect_dir"

browser_line="$(grep -n -- '- name: Run browser smoke against staging' "$repo_root/.github/workflows/scope-railway-staging.yml" | cut -d: -f1)"
git_line="$(grep -n -- '- name: Run Git router smoke against staging' "$repo_root/.github/workflows/scope-railway-staging.yml" | cut -d: -f1)"
test "$browser_line" -lt "$git_line"
