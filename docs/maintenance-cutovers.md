# Maintenance migration cutovers

Migration impact is declared once in `crates/scope-postgres/src/migrations/mod.rs`.
Use `Online` only when the old and new runtime contracts can safely overlap.
Renames, removals, rewrites, protocol resets, and changed invariants require
`MaintenanceRequired`. Do not add dual readers or writers to avoid a cutover.

Production migration ownership belongs to the backend deployment job. For a
maintenance-required plan it:

1. Records the API and worker replica counts from Railway's current service
   configuration. A first deployment uses the explicit replica counts from the
   workflow because Railway has no deployment-derived replica metadata yet.
2. Stops the exact active API deployment through Railway's deployment API,
   then stops the exact active worker deployment, reading back zero running
   replicas for each.
3. Runs `scope-maintenance apply` once. The command also refuses unless it can
   acquire the database's exclusive writer fence.
4. Verifies the exact migration ledger while traffic remains closed, then runs
   `scope-maintenance backfill-landing-files` with the API service's object-store
   configuration. The command is idempotent and verifies each row against the
   current live-file metadata before it commits.
5. Uploads and health-checks the worker from the checked-out revision, then
    uploads and health-checks the API from that same revision.

`backfill-landing-files` and its deploy hook are temporary m0026 cutover code. Remove both after production has completed the migration and a rerun reports zero rows to backfill.

The successful migration transaction is the point of no return. After a failed
apply command, the workflow redeploys the exact stopped worker and API deployment
IDs only when a fresh ledger read proves that the pre-migration state is unchanged.
A committed or unreadable/otherwise indeterminate ledger leaves both writer
services closed and must be recovered by rerunning the same revision to finish
the forward deployment. Never restore an old API or worker unless rollback is
positively proven.

The database fence is the final concurrency boundary, not Railway's replica
readback. Every API and worker database connection holds the shared side of the
fence for its session. A deployment that races the shutdown either makes the
maintenance command refuse before migration, or blocks while the exclusive
fence is held. Runtime startup verifies the exact schema before opening its
writer pool, so an old binary cannot begin writing after a committed migration.

The maintenance command supports read-only inspection:

```text
scope-maintenance plan
scope-maintenance verify
```

Both emit JSON. `verify` succeeds only when the database ledger exactly matches
the binary; it rejects both missing and unknown migration versions.
