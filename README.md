# Scope VCS

Scope is an ACL-aware source-control core with Git-compatible projections.

The v1 promise is narrow and testable: a principal only receives the paths,
objects, metadata, and history they are authorized to see. Git is an adapter;
the canonical source of truth is a server-side source graph.

## Workspace

- `crates/scope-policy` - top-down path visibility and authorization.
- `crates/scope-projection` - canonical commits to per-principal projections.
- `crates/scope-git` - projected Git object identity helpers.
- `crates/scope-crypto` - signed push manifest primitives.
- `crates/scope-store` - storage contracts, demo data, and colocated SQL schema.
- `crates/scope-server` - Axum API and Git facade boundary.
- `crates/scope-cli` - `sx` command-line prototype.
- `crates/scope-worker` - background worker placeholder.
- `apps/web` - TanStack Start control-plane UI.

## Local Checks

```bash
cargo test --workspace
pnpm install
pnpm build
```

## Deployment Shape

Railway services:

- `scope-api` is a Railpack Rust service. It builds the root `scope-vcs`
  binary and starts `./bin/scope-vcs`.
- `scope-web` is a Railpack Node service. Because the repo root is also a
  Rust workspace, set `RAILPACK_CONFIG_FILE=railpack.web.json` on this
  Railway service so Railpack uses the web-specific Node provider config.
- `scope-worker` is not deployed for the first version.

Railway Postgres stores canonical metadata. Railway Buckets store encrypted
source blobs/chunks; app-layer encryption remains mandatory.
