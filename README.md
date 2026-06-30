# Scope VCS

Scope is an ACL-aware source-control core with Git-compatible projections.

The v1 promise is narrow and testable: a principal only receives the paths,
objects, metadata, and history they are authorized to see. Git is an adapter;
the canonical source of truth is a server-side source graph.

## Layout

- `api` - Axum API, Git facade boundary, and API-owned domain modules for
  policy, projection, Git projection, and catalog state.
- `cli` - Rust `scope` CLI plus the Railway installer service that serves
  generated install scripts and CI-built CLI binaries.
- `web` - TanStack Start control-plane UI.

## Local Checks

```bash
(cd api && cargo fmt -- --check)
(cd api && cargo test)
(cd api && cargo clippy --all-targets --locked -- -D warnings)
(cd cli && cargo fmt -- --check)
(cd cli && cargo test)
(cd cli && cargo clippy --all-targets --locked -- -D warnings)
(cd cli && cargo build --release --locked)
(cd web && pnpm install)
(cd web && pnpm check)
(cd web && pnpm build)
(cd web && pnpm test)
```

`pnpm check` runs from `web` because it uses Node. It typechecks the web app,
checks the Rust/TypeScript API response contract, and scans the whole repo for
source files over 1,000 lines while ignoring generated files, lockfiles,
dependencies, and build output.

## Local Development

Use the committed dev entrypoint instead of pulling Railway variables by hand:

```bash
./dev/scope-dev doctor
./dev/scope-dev up
./dev/scope-dev bench
./dev/scope-dev status
./dev/scope-dev down
./dev/scope-dev reset
```

The local stack runs the web app at `http://localhost:3000` and the API at
`http://localhost:8080`. The API is started with `--features local-dev`, uses
ephemeral in-memory metadata seeded with local demo repositories, and stores
encrypted local objects under `.scope/dev`. The dev launcher strips inherited
Railway variables and refuses production-looking Clerk or database settings.

`web/.env.local` must contain Clerk development keys (`pk_test_` and
`sk_test_`). The launcher derives the local API Clerk issuer from
`VITE_CLERK_PUBLISHABLE_KEY`, so local web and local API stay on the same Clerk
development instance.

Root `.env.local` must contain `SCOPE_DEV_USER_EMAIL` matching the Clerk dev
account you sign in with. `SCOPE_DEV_USER_HANDLE` is optional and controls the
owner handle for seeded repositories. Copy `.env.example` when setting this up
on a new machine.

`./dev/scope-dev bench` runs the Phase 0 local data-architecture benchmark from
`bench/` against the seeded repos and writes ignored JSON/Markdown reports under
`.tmp/bench/phase0/`. It reuses the local `scope_cli_...` session token when
available, and `SCOPE_BENCH_AUTH_TOKEN` can override that auto-detected auth.
The mutating Git receive-pack path uses throwaway repos created and deleted by
the benchmark harness.

Do not use Railway `production` variables for local API development. A deployed
Railway development environment can be added later for integration testing, but
the default local workflow is intentionally hermetic and disposable.

## Deployment Shape

Railway services:

- `scope-api` is a Railpack Rust service rooted at `api`. Build and start the
  `api` binary from that directory. It requires `DATABASE_URL` from the
  Railway Postgres service and runs API-owned SeaORM migrations on startup.
  Keep the API service port pinned to `8080` if `scope-web` uses the private
  URL example below.
- `scope-cli` is a Railpack Rust service rooted at `cli`. Runtime deploys build
  `scope-cli-service`, which serves `/install.sh`, `/install.ps1`, and
  allowlisted files from `/downloads/<artifact>`. The downloadable `scope`
  binaries are built by `.github/workflows/scope-cli-build.yml`, staged in
  `cli/dist`, checksummed, copied into `.railway-upload/cli`, and uploaded to
  Railway with `railway up .railway-upload --path-as-root --no-gitignore`.
  The service `readyz` route
  stays unavailable until every manifest artifact and checksum exists in
  `SCOPE_CLI_ARTIFACT_DIR`, which defaults to `./dist`.
  Supported install targets are Linux x64, Linux ARM64, macOS Intel, macOS
  Apple Silicon, Windows x64, and Windows ARM64. Raspberry Pi and Alpine builds
  are intentionally not published yet. Keep the service linked to GitHub for
  source metadata, but treat the GitHub Actions workflow as the authoritative
  deploy path because a plain Railway source deploy does not include CI-built
  cross-platform artifacts. Use the Railway-generated public service URL as
  `SCOPE_CLI_INSTALL_URL` in `scope-web`. Set `SCOPE_CLI_PUBLIC_URL` on the CLI
  service to the same public URL when you want installer scripts to avoid
  inferring the URL from request headers.
- `scope-web` is a Railpack Node service rooted at `web`. Build and CI use
  Node 24. Railway runs the Vite production build only; GitHub Actions owns
  typechecking and other checks. Railpack caches the pnpm store between web
  deploys. The service requires two API origin variables, the CLI installer
  origin, and Clerk browser/server keys:
  - `SCOPE_API_INTERNAL_URL` is the server-to-server API origin used by
    TanStack Start server functions. On Railway, point it at the API private
    domain with the API port, for example
    `http://${{scope-api.RAILWAY_PRIVATE_DOMAIN}}:8080`.
  - `SCOPE_API_PUBLIC_URL` is the public API origin returned to the CLI for Git
    commands, for example `https://scope-api-production-0251.up.railway.app`.
    Do not point this at a `railway.internal` host; browsers and local Git
    clients cannot reach Railway's private network.
  - `SCOPE_CLI_INSTALL_URL` is the public `scope-cli` Railway URL used for the
    empty-home install command, for example
    `https://${{scope-cli.RAILWAY_PUBLIC_DOMAIN}}`.
  - `VITE_CLERK_PUBLISHABLE_KEY`, `CLERK_SECRET_KEY`, and the Clerk redirect
    URL variables generated by `clerk init --framework tanstack-start`.
  The web app intentionally has no hard-coded production API fallback.
- `scope-api` also requires `SCOPE_API_PUBLIC_URL`, `SCOPE_APP_ORIGIN`,
  `CLERK_ISSUER`, and optionally `CLERK_JWKS_URL`, `CLERK_AUTHORIZED_PARTIES`,
  and `CLERK_AUDIENCE`. The API verifies Clerk API tokens against the
  configured issuer, authorized origins, and the API audience. The default API
  audience is `scope-api`; set `CLERK_AUDIENCE` only to override that value.
  `CLERK_AUTHORIZED_PARTIES` defaults to `SCOPE_APP_ORIGIN` when omitted.
- Clerk development and production instances must each define a JWT template
  named `scope_api` with an `aud` claim of `scope-api`. The web app requests
  this template for server-side API calls; missing templates cause Clerk token
  generation to fail before the Scope API is reached.

Railway Postgres stores canonical metadata. Railway Buckets store encrypted
source blobs and Git bundle snapshots.

GitHub Actions deploy variables:

- `RAILWAY_TOKEN` - repository secret used by service workflows to deploy to
  Railway after service-specific checks pass on `main`.
- `RAILWAY_PROJECT_ID` - repository secret for the Railway project that owns
  the `scope-api`, `scope-cli`, and `scope-web` services.

Railway GitHub autodeploy should stay disabled for app services when CI owns
deploys. The service-specific workflows pass the pushed commit message to
Railway with `railway up --message`, so deployment names track the merged PR or
commit rather than the raw CLI command.

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
