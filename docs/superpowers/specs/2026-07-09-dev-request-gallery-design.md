# Dev Request Gallery Design

## Goal

Make the local `update-demo` repository exercise the completed request-review UI without adding production behavior or decorative records that violate request invariants.

## Gallery

Seed four owner-authored requests:

- `Submitted`: ready for maintainer review and merge.
- `NeedsResponse`: includes maintainer feedback and waits on the contributor.
- `Resolved` with `Accepted`: demonstrates a successfully merged request and settlement timeline.
- `Withdrawn`: demonstrates a contributor-closed request.

Each request has distinct copy, timestamps, events, and a real changed file so the list, Overview, Changes, file-diff, and Activity surfaces contain useful data.

## Architecture

Keep the implementation in the local-dev seed boundary under `dev/api`. Build one deterministic temporary Git repository for `update-demo`, create request refs from the appropriate main revision, and store real bundles in the existing object store.

Drive request state through the existing domain functions for start, upload, submit, feedback, merge, and withdrawal. Do not construct request state or event ledgers by hand. The accepted request updates the seeded main snapshot before later requests branch, so base and head OIDs remain coherent.

No web components, production routes, compatibility paths, or general fixture framework are added.

## Verification

- Extend the focused local-dev seed tests to assert the four states and restorable request refs.
- Run formatting, the full Rust workspace suite on remote Linux, and existing web checks.
- Reset the remote Linux dev seed and inspect the request list plus each request's Overview, Changes, and Activity views.
- Perform a subtraction review before committing implementation.

## Delivery

Commit the seed work on `codex/sco-65-ui-audit-plan`, push it, open a pull request, and monitor CI and review feedback until all actionable findings are resolved or an external blocker requires user input.
