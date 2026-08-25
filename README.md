# Scope

Scope is a pre-alpha source-control platform built around permissioned repository
projections. Maintainers keep one repository, choose which files are public, and
share only that public projection with outside contributors.

Each repository carries trusted `.scope/RULES.md` guidance. Contributor changes
cannot modify protected `.scope` paths. Contribution requests move through a
small Draft, Open, Closed, and Merged lifecycle: drafts stay private to their
participants, and submission places work in the maintainer queue without making
the request immutable.

Scope does not use staking, credits, or automated qualification to rank or admit
requests. Verified participants can rate one another after a request closes or
merges, but ratings do not grant permissions or control submission.

## Repository map

- `api/` owns HTTP delivery, authentication, server-sent events, and application
  composition.
- `worker/` owns durable background work, cloud execution, cleanup, and Git
  compaction.
- `cache-service/` serves workflow-cache metadata and signed object transfers.
- `runner-runtime/` executes isolated workflow jobs.
- `cli/` is the independently built and released command-line client.
- `web/` contains the browser application and generated API contract.
- `crates/` contains domain code, wire contracts, and infrastructure adapters.
- `dev/` contains the supported local-development and check entrypoints.
- `bench/` contains storage and deployed-system benchmarks.

## Local development

Copy the environment templates and add a matching Clerk development key pair to
`web/.env.local`:

```bash
cp .env.example .env.local
cp web/.env.example web/.env.local
./dev/scope-dev doctor
./dev/scope-dev up
```

The local web app runs at `http://localhost:3000`; the API runs at
`http://localhost:8080`. Run `./dev/scope-dev --help` for setup requirements,
commands, safety checks, and state locations.

## Checks

Run the complete non-browser check suite from the repository root:

```bash
./dev/check
```

Use `./dev/check --help` to run one check group. Browser smoke requires the local
stack and Clerk development credentials:

```bash
./dev/check smoke
```

Benchmark methodology is documented in [`bench/README.md`](bench/README.md).
Current technical ownership is documented in
[`docs/architecture.md`](docs/architecture.md).
Production migration recovery is documented in
[`docs/maintenance-cutovers.md`](docs/maintenance-cutovers.md).
