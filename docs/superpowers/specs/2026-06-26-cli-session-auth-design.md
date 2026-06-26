# CLI Session Auth PR 1 Design

## Scope

This PR makes CLI auth machine-scoped instead of repo-init-scoped.

Included:

- durable Postgres-backed CLI sessions
- OS keychain storage for the active CLI session token
- `scope login`
- `scope logout`
- `scope whoami`
- `scope init` reusing cached auth before starting any browser flow
- tests for session validation, revocation, expiry, and init auth selection

Not included:

- localhost browser callback login
- web-generated one-time exchange commands for agents
- web CLI session management UI

Those belong in the next auth UX PR after the durable session boundary exists.

## Target Flow

```text
scope login
  -> opens the existing Clerk-backed device login flow
  -> stores the returned Scope CLI session token in the OS keychain

scope init
  -> reads cached CLI session
  -> validates it with the API
  -> creates the repository immediately when valid
  -> falls back to login when missing, expired, or revoked
```

Repo Git credentials remain repo-specific and separate from CLI API auth.

## API Shape

The API keeps the current device-login endpoints as the login transport, but the issued token becomes a durable CLI session instead of a short one-off token.

New or updated behavior:

- CLI session rows store a token hash, `user_id`, creation time, last-used time, expiry, revocation time, and a display label.
- token verification rejects expired or revoked sessions.
- successful token verification updates `last_used_at`.
- CLI callers can validate the active session through an authenticated account endpoint.
- users can revoke all or individual CLI sessions in a later UI without changing the token model.

## CLI Shape

The CLI gains an auth boundary around API access:

- `scope login` runs the existing browser/device login and stores the token.
- `scope logout` deletes the stored token.
- `scope whoami` validates the token and prints the resolved Scope user.
- `scope init` calls the auth boundary first and only launches login if the cached token cannot be used.

Local storage uses the OS keychain. If keychain access fails, the CLI fails clearly rather than writing plaintext credentials to disk.

## Testing

Focused coverage should prove:

- valid CLI sessions authenticate API requests.
- expired sessions are rejected.
- revoked sessions are rejected.
- login tokens are stored only after a successful device-login exchange.
- logout removes the stored token.
- `scope init` skips login when cached auth is valid.
- `scope init` falls back to login when cached auth is missing or invalid.

## Commit Plan

1. Add this design document.
2. Add API durable CLI session fields and tests.
3. Add CLI keychain auth commands.
4. Make `scope init` reuse cached auth.
5. Run checks, push, open PR, and run the autoreview loop.
