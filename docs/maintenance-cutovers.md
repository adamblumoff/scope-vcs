# Maintenance migration recovery

Migration impact is declared once in `crates/scope-postgres/src/migrations/mod.rs`.
Use `Online` only when old and new runtime contracts can overlap safely. Renames,
removals, rewrites, protocol resets, and changed invariants require
`MaintenanceRequired`. Do not add dual readers or writers to avoid a cutover.

The production backend deployment workflow owns the cutover procedure. It checks
the migration plan, validates the Railway target, closes the API and worker,
acquires the database writer fence, applies migrations, verifies the exact
ledger, runs the current idempotent backfills, and deploys worker and API builds
from the same revision. Keep exact command ordering and the active backfill list
in `.github/scripts/deploy-backend-railway.sh`, where the deployment tests can
exercise them.

## Recovery rule

The successful migration transaction is the point of no return. If `apply`
fails, the workflow reads the ledger again. It restarts the stopped release only
when that read proves the pre-migration ledger is unchanged.

If the ledger is exact for the new binary, unreadable, or otherwise
indeterminate, the workflow keeps both writers closed. Rerun the same revision
to finish the forward deployment. Never restart an old API or worker unless the
fresh ledger read proves that the migration did not commit.

The database fence is the final concurrency boundary, not Railway replica
readback. Every API and worker database connection holds the shared side for its
session. A writer racing shutdown either makes maintenance refuse or drains
before the exclusive fence is acquired. Runtime startup verifies the exact
schema before opening its writer pool, so an old binary cannot start writing
after a committed migration.

## Read-only inspection

```text
scope-maintenance plan
scope-maintenance verify
```

Both commands emit JSON. `verify` succeeds only when the database ledger exactly
matches the binary. Run `scope-maintenance --help` for the current maintenance
and backfill commands. Production cutovers should use the deployment workflow,
not a hand-written command sequence.
