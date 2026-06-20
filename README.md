# Scope VCS

Scope is an ACL-aware source-control core with Git-compatible projections.

The v1 promise is narrow and testable: a principal only receives the paths,
objects, metadata, and history they are authorized to see. Git is an adapter;
the canonical source of truth is a server-side source graph.

## Workspace

- `crates/scope-server` - Axum API, Git facade boundary, and server-owned
  domain modules for policy, projection, Git projection, and catalog state.
- `apps/web` - TanStack Start control-plane UI.

## Local Checks

```bash
cargo test --workspace
pnpm install
pnpm build
pnpm test:web
pnpm check:line-limit
```

`pnpm check:line-limit` fails any source file over 1,000 lines while ignoring
generated files, lockfiles, dependencies, and build output. It is intentionally
not part of `pnpm check` yet because current `main` still has the unsplit server
entrypoint over the limit; wire it into the root check after the backend split
lands.

## Deployment Shape

Railway services:

- `scope-api` is a Railpack Rust service. Build the `scope-server` package and
  start its `scope-server` binary.
- `scope-web` is a Railpack Node service. Because the repo root is also a
  Rust workspace, set `RAILPACK_CONFIG_FILE=railpack.web.json` on this
  Railway service so Railpack uses the web-specific Node provider config.
- `scope-web` also requires `VITE_SCOPE_API_URL` to point at the deployed
  `scope-api` origin. The web app intentionally has no hard-coded production
  API fallback.

Railway Postgres stores canonical metadata. Railway Buckets store encrypted
source blobs/chunks; app-layer encryption remains mandatory.
