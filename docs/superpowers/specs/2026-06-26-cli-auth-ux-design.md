# CLI Auth UX PR 2 Design

## Scope

This PR makes CLI login easier after durable CLI sessions exist.

Included:

- localhost browser-callback login as the default `scope login` path
- `scope login --headless` keeping the existing device-code flow
- `scope login --exchange <scope_otc_...>` for agent-friendly login
- one-time web-generated exchange commands for signed-in users
- basic Scope-owned CLI session UI: current sessions and revoke actions
- tests for callback, exchange, one-time use, expiry, and revocation

Not included:

- OAuth provider changes in Clerk
- long-lived refresh-token rotation
- organization/team auth
- full account settings redesign
- custom Clerk sign-in implementation

Clerk remains responsible for web identity. Scope remains responsible for CLI
session issuance, exchange grants, session storage, and session revocation.

## Target Flow

```text
scope login
  -> CLI starts a temporary localhost callback listener
  -> CLI asks API to create a browser login request
  -> CLI opens Scope web authorization URL
  -> web page requires Clerk auth and approves the request
  -> browser redirects to localhost with a one-time callback code
  -> CLI exchanges callback code for a durable Scope CLI session
  -> CLI stores the session token in the OS keychain

scope login --headless
  -> uses the existing device-code flow
  -> stores the session token in the OS keychain

scope login --exchange scope_otc_...
  -> exchanges a one-time web-generated token for a durable CLI session
  -> stores the session token in the OS keychain
```

`scope init` keeps the PR 1 behavior: validate cached auth first, then invoke
login only when needed.

## API Shape

New Scope-owned server state:

- `scope_cli_browser_logins`
  - request id
  - request secret hash
  - callback URL
  - callback code hash
  - created/expires/completed/consumed timestamps
  - completed user id
- `scope_cli_exchange_grants`
  - grant hash
  - user id
  - created/expires/consumed timestamps

New API endpoints:

- `POST /v1/cli/browser-login`
  - unauthenticated CLI start
  - accepts localhost callback URL
  - returns authorization URL and request secret
- `POST /v1/cli/browser-login/{request_id}/complete`
  - Clerk-authenticated web approval
  - creates a one-time callback code and marks the request complete
- `POST /v1/cli/browser-login/{request_id}/exchange`
  - unauthenticated CLI exchange
  - requires request secret and callback code
  - returns a durable CLI session token once
- `POST /v1/cli/exchange-grants`
  - Clerk-authenticated web action
  - returns one plaintext `scope_otc_...` grant once
- `POST /v1/cli/exchange-grants/exchange`
  - unauthenticated CLI exchange
  - consumes the grant and returns a durable CLI session token once
- `GET /v1/cli/sessions`
  - Clerk-authenticated web list of current CLI sessions
- `DELETE /v1/cli/sessions/{session_id}`
  - Clerk-authenticated revoke for a user's own CLI session

All secrets are stored only as hashes. Plaintext callback codes, exchange
grants, and CLI session tokens are returned once.

## Web Shape

`/cli-login` becomes a signed-in authorization surface:

- browser callback request: show terminal authorization context and an approve
  button, then redirect to the localhost callback URL
- headless fallback: preserve manual device-code entry
- no generic token/debug language

A small `/account` route owns CLI session management:

- show the signed-in user chrome with Clerk's existing `UserButton`
- generate a one-time exchange command
- list active CLI sessions
- revoke individual sessions

The UI stays minimal, dense, and product-focused. It should use the existing
components and avoid decorative cards.

## CLI Shape

`scope login` attempts the browser-callback flow first:

- bind `127.0.0.1` on a random free port
- start browser login with the callback URL
- open the returned authorization URL
- wait for one localhost callback request
- exchange the callback code for a durable session token
- store the token in the OS keychain

Fallbacks:

- `scope login --headless` uses device-code login directly
- `scope login --exchange <grant>` uses web-generated one-time grants

## Testing

API:

- browser login request accepts only localhost callback URLs
- browser login completion requires Clerk auth
- callback code exchange is one-time use
- expired browser login fails
- exchange grants are one-time use and expire
- users can list and revoke only their own CLI sessions

CLI:

- login command parses default, headless, and exchange modes
- exchange mode stores a returned session token
- callback URL construction stays loopback-only

Web:

- signed-out authorization redirects to sign-in
- signed-in exchange command generation works
- session list renders empty and non-empty states

## Commit Plan

1. Add this design document.
2. Add API browser-login, exchange-grant, and session-management endpoints.
3. Add web CLI authorization and account/session UI.
4. Add CLI callback and exchange login modes.
5. Run checks, push, open PR, and run the autoreview loop.
