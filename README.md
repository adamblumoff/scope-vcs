# Scope VCS

Scope is an ACL-aware source-control core with Git-compatible projections.

The v1 promise is narrow and testable: a principal only receives the paths,
objects, metadata, and history they are authorized to see. Git is an adapter;
the canonical source of truth is a server-side source graph.

## Layout

- `api` - Axum API, Git facade boundary, and API-owned domain modules for
  policy, projection, Git projection, and catalog state.
- `web` - TanStack Start control-plane UI.

## Local Checks

```bash
(cd api && cargo test)
(cd web && pnpm install)
(cd web && pnpm check)
(cd web && pnpm build)
(cd web && pnpm test)
```

`pnpm check` runs from `web` because it uses Node. It typechecks the web app,
checks the Rust/TypeScript API response contract, and scans the whole repo for
source files over 1,000 lines while ignoring generated files, lockfiles,
dependencies, and build output.

## Deployment Shape

Railway services:

- `scope-api` is a Railpack Rust service rooted at `api`. Build and start the
  `api` binary from that directory. It requires `DATABASE_URL` from the
  Railway Postgres service and runs API-owned SeaORM migrations on startup.
  Keep the API service port pinned to `8080` if `scope-web` uses the private
  URL example below.
- `scope-web` is a Railpack Node service rooted at `web`. It requires two API
  origin variables:
  - `SCOPE_API_INTERNAL_URL` is the server-to-server API origin used by
    TanStack Start server functions. On Railway, point it at the API private
    domain with the API port, for example
    `http://${{scope-api.RAILWAY_PRIVATE_DOMAIN}}:8080`.
  - `SCOPE_API_PUBLIC_URL` is the public API origin shown in Git remote/setup
    commands, for example `https://scope-api-production-0251.up.railway.app`.
    Do not point this at a `railway.internal` host; browsers and local Git
    clients cannot reach Railway's private network.
  The web app intentionally has no hard-coded production API fallback.

Railway Postgres stores canonical metadata. Railway Buckets store encrypted
source blobs and Git bundle snapshots.

`scope-api` bucket variables:

- `SCOPE_BUCKET_ENDPOINT` - Railway bucket `ENDPOINT`, for example
  `https://storage.railway.app`.
- `SCOPE_BUCKET_NAME` - Railway bucket `BUCKET`.
- `SCOPE_BUCKET_REGION` - Railway bucket `REGION`, commonly `auto`.
- `SCOPE_BUCKET_ACCESS_KEY_ID` - Railway bucket `ACCESS_KEY_ID`.
- `SCOPE_BUCKET_SECRET_ACCESS_KEY` - Railway bucket `SECRET_ACCESS_KEY`.
- `SCOPE_BUCKET_FORCE_PATH_STYLE` - optional. Leave unset for current Railway
  virtual-hosted-style buckets. Set to `true` only when the bucket credentials
  tab says the bucket needs path-style URLs.
- `SCOPE_OBJECT_ENCRYPTION_KEY` - required app-layer encryption key for bucket
  objects. Generate 32 random bytes and store them as base64, for example with
  `openssl rand -base64 32`.
