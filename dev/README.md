# Local Dev

`dev/scope-dev.ps1` is the supported local development entrypoint.
`dev/api` contains Rust code that is compiled only for `--features local-dev`;
it is intentionally outside `api/src` so it is not part of the production API
runtime.

```powershell
.\dev\scope-dev.ps1 doctor
.\dev\scope-dev.ps1 up
.\dev\scope-dev.ps1 status
.\dev\scope-dev.ps1 down
.\dev\scope-dev.ps1 reset
```

The command starts:

- web at `http://localhost:3000`
- API at `http://localhost:8080`

Local API runs with `cargo run --features local-dev`, in-memory metadata, and
filesystem object storage under `.scope/dev`. The script strips inherited
Railway variables and refuses production-looking Clerk or database settings.

`web/.env.local` must contain Clerk development keys. The script derives
`CLERK_ISSUER` for the local API from `VITE_CLERK_PUBLISHABLE_KEY`.
