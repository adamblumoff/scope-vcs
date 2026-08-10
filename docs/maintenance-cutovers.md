# Maintenance migration cutovers

Migration impact is declared once in `crates/scope-postgres/src/migrations/mod.rs`.
Use `Online` only when the old and new runtime contracts can safely overlap.
Renames, removals, rewrites, protocol resets, and changed invariants require
`MaintenanceRequired`. Do not add dual readers or writers to avoid a cutover.

Production migration ownership belongs to the backend deployment job. For a
maintenance-required plan it:

1. Records the API and worker replica topology from their latest successful
   deployments.
2. Scales API traffic to zero, then scales the worker to zero, reading back
   zero running replicas for each.
3. Runs `scope-maintenance apply` once. The command also refuses unless it can
   acquire the database's exclusive writer fence.
4. Uploads API and worker artifacts from the same checked-out revision and
   verifies the exact migration ledger while traffic remains closed.
5. Starts and health-checks the worker, then starts and health-checks the API.

The successful migration transaction is the point of no return. After a failed
apply command, the workflow restores the recorded old topology only when a
fresh ledger read proves that the pre-migration state is unchanged. A committed
or unreadable/otherwise indeterminate ledger leaves both writer services closed
and must be recovered by rerunning the same revision to finish the forward
deployment. Never restore an old API or worker unless rollback is positively
proven.

The maintenance command supports read-only inspection:

```text
scope-maintenance plan
scope-maintenance verify
```

Both emit JSON. `verify` succeeds only when the database ledger exactly matches
the binary; it rejects both missing and unknown migration versions.
