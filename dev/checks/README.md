# Check entrypoints

These scripts own the check commands used by `dev/check`, GitHub Actions, and
`.scope/runs/checks.yml`. They run from the repository root, regardless of the
caller's working directory.

| Entrypoint | Coverage |
| --- | --- |
| `backend with-api` | Workspace formatting, tests, API test support, local development helpers, Clippy |
| `backend without-api` | Workspace formatting, tests and Clippy excluding API |
| `cli` | Standalone formatting, tests, distribution selector, Clippy, both release binaries |
| `web` | Tests, types, generated API contract, observer boundary, React health, structure, build |
| `contract` | Generated API TypeScript and validator comparison |
| `policy` | Complete-tree source size, Rust boundaries, toolchain pins, gate inventory |
| `integration web` | Browser smoke against a running seeded stack |
| `integration cli` | Opt-in two-actor contribution flow against a running seeded stack |
| `ops` | Deployment, staging, benchmark, and AWS infrastructure tests |

Callers install Rust, Node and pnpm dependencies, configure databases and secrets,
and start/stop integration stacks. The contract check needs Rust and web
dependencies. CLI integration requires `SCOPE_CLI_E2E=1` and `SCOPE_API_URL`.
GitHub retains native distribution build matrices; these scripts do not select
platforms or provision credentials. Repository policy always checks the full
checkout, including source outside `web/`.
