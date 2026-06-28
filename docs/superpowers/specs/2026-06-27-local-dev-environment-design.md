# Local Dev Environment Redesign

## Problem

Local development currently depends on operator-specific shell commands and ignored
`.tmp` files. The API can be started with production Railway variables and then
manually patched to accept local Clerk tokens. That mixes development auth with
production data and makes local bugs hard to reason about.

## Goals

- One command starts the local web and API stack.
- Local API startup refuses dangerous environment combinations.
- Local development uses disposable local metadata and local object storage by
  default.
- Local memory metadata starts with seeded repositories for UI development.
- Clerk development keys are the only supported local auth keys.
- The development contract is versioned in the repo, not stored in shell history.
- Old ad hoc local-dev scripts and docs are removed or replaced.
- Scope users are unique by normalized verified email.

## Runtime Shape

Local development has three boundaries:

1. Web runs at `http://localhost:3000` with Clerk development keys.
2. API runs at `http://localhost:8080` with a matching Clerk development issuer.
3. Persistence is local: ephemeral seeded in-memory metadata by default,
   optional local Postgres for integration work, and filesystem object storage
   under `.scope/dev`.

Railway production variables are not part of the default local workflow. A
separate Railway development environment may be added later for deployed
integration testing, but local development must stay safe without it.

Local-only implementation code must be visually separated from the production
runtime:

- Repository orchestration lives under top-level `dev/`.
- API local-dev startup code lives under top-level `dev/api/` and is compiled
  into the API only with the `local-dev` feature.
- Production `AppState::from_env()` remains Postgres plus encrypted S3 object
  storage and does not branch on local dev variables.

## Developer Commands

Add a committed `dev` command surface:

- `up` starts API and web, writes logs and pid files under `.tmp/local-dev`.
- `down` stops only processes owned by those pid files.
- `status` reports process and readiness state.
- `doctor` checks required tools, ports, env shape, database safety, and Clerk
  issuer alignment.
- `reset` resets local runtime data after explicit operator action.

The launcher should keep local defaults small and explicit. It may read
`.env.local` and `web/.env.local`, but it must not pull production Railway
variables.

The default local stack does not start the CLI installer service. UI development
uses preloaded repositories instead of depending on `scope init` or
`scope-cli-service`.

## API Changes

Add first-class local runtime dependencies:

- `SCOPE_METADATA_STORE=memory` keeps local UI work zero-dependency and
  disposable, with seeded demo repositories.
- `SCOPE_METADATA_STORE=postgres` uses `DATABASE_URL` when a local Postgres is
  available.
- Omitted `SCOPE_METADATA_STORE` defaults to `memory` when `SCOPE_ENV=local`
  and `postgres` otherwise.

Add first-class local object storage through the existing `ObjectStore` trait:

- `SCOPE_OBJECT_STORE=s3` keeps production behavior.
- `SCOPE_OBJECT_STORE=filesystem` stores encrypted object envelopes on disk.
- Omitted `SCOPE_OBJECT_STORE` defaults to `s3` unless `SCOPE_ENV=local`.

Add environment validation before `AppState` starts:

- `SCOPE_ENV=local` requires localhost app/API origins.
- `SCOPE_ENV=local` rejects Railway production markers.
- `SCOPE_ENV=local` rejects database URLs without a visible local-dev marker
  when `SCOPE_METADATA_STORE=postgres`.
- `SCOPE_ENV=local` rejects live Clerk keys and production Clerk issuers.
- `SCOPE_ENV=local` requires `SCOPE_DEV_USER_EMAIL` so seeded repositories attach
  to the signed-in Clerk dev account by verified email.

## Seeded Data

Local memory metadata creates one Scope user from `SCOPE_DEV_USER_EMAIL` and
optional `SCOPE_DEV_USER_HANDLE`. The seed includes a published repository, a
pending publish review repository, and a published repository with a staged
update. The seed writes matching sample file blobs through the local encrypted
object store so repo detail and review screens load real content.

## Auth Identity Rule

Scope users are unique by normalized verified email. If a new Clerk subject logs
in with an email already owned by a Scope user, the new auth identity links to
that existing Scope user instead of creating a second user. Unverified or missing
emails do not merge and should fail where a verified email is required.

The database schema enforces the invariant with a unique email index. Because the
product is pre-alpha, destructive schema reset on drift is acceptable and expected.

## Verification

Implementation is complete only when:

- `dev doctor` passes in the local environment.
- `dev up` starts API and web without Railway production variables, local
  Postgres, or a CLI installer service.
- Signed-in local web shows seeded repositories without running `scope init`.
- `/readyz` passes on the local API.
- `dev down` stops owned processes.
- API tests cover local object storage, environment guards, and same-email Clerk
  identity merging.
- Existing API, CLI, and web checks pass or any remaining failure is documented
  with a concrete blocker.
