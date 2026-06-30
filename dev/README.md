# Local Dev

`dev/scope-dev` is the supported local development entrypoint.
`dev/api` contains Rust code that is compiled only for `--features local-dev`;
it is intentionally outside `api/src` so it is not part of the production API
runtime.

```bash
./dev/scope-dev doctor
./dev/scope-dev up
./dev/scope-dev bench
./dev/scope-dev status
./dev/scope-dev down
./dev/scope-dev reset
```

The command starts:

- web at `http://localhost:3000`
- API at `http://localhost:8080`

Local API runs with `cargo run --features local-dev`, in-memory metadata seeded
with demo repositories, and filesystem object storage under `.scope/dev`. The
script strips inherited Railway variables and refuses production-looking Clerk
or database settings.

`web/.env.local` must contain Clerk development keys. The script derives
`CLERK_ISSUER` for the local API from `VITE_CLERK_PUBLISHABLE_KEY`.

Root `.env.local` must contain `SCOPE_DEV_USER_EMAIL` matching the Clerk dev
account you sign in with. `SCOPE_DEV_USER_HANDLE` is optional.

The local dev stack intentionally does not start the CLI installer service.
Seeded repositories are the default UI development path until a separate CLI dev
environment exists.

Run `./dev/scope-dev bench` while the stack is up to collect the Phase 0 local
data-architecture baseline. The benchmark harness lives under `bench/`, uses
the seeded local repos, reuses the local Scope CLI session when available, and
writes ignored reports to `.tmp/bench/phase0/`.
