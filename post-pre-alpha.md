# Post-Pre-Alpha Checklist

These are deliberate pre-alpha shortcuts and deferred architecture decisions to
revisit before Scope carries production data or real users.

## Data Durability

When this product leaves pre-alpha, revisit destructive metadata and schema
reset behavior. Automatic startup resets are acceptable for now, but future
production data should use explicit migrations, backfills, backups, or fail-fast
operator action instead of silently deleting repo metadata after persisted shape
changes.

## Commit History

- Decide whether history is only for live published projections or also for
  pending review projections. If review history becomes a product surface, make
  the API contract explicit with a source dimension instead of pointing review
  commits at live history.
- Keep commit history URLs shareable. New history interactions should preserve
  enough search state to reopen the selected audience and commit.
- If repos grow large enough for history reads to matter, replace per-request
  projection rebuilds with a domain-owned read model or index that can serve
  list, detail, and file-diff lookups without recomputing the whole projection.
- If history response types keep growing, split them into a behavior-owned
  response module instead of letting the shared HTTP response file become a
  catch-all.
- Decide whether file visibility in history means current visibility or
  visibility at that commit. If this becomes audit history, model the
  at-commit visibility explicitly.
