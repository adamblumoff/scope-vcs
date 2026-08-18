# Scope Cloud execution on Northflank

Scope uses one Northflank manual job as a reusable run-once primitive. The worker starts a fresh
Northflank run for each queued workflow job and overrides the job with the workflow's immutable
image digest. Northflank retries are disabled because Scope owns retry and idempotency semantics.

## One-time setup

1. Build and publish a Linux amd64 workflow image containing
   `/scope/bin/scope-runner-runtime`. The checks image is the reference implementation.
2. Choose a Northflank deployment plan with roughly 4 vCPU and 16 GiB RAM.
3. Export `NORTHFLANK_API_TOKEN`, `NORTHFLANK_PROJECT_ID`, `NORTHFLANK_DEPLOYMENT_PLAN`, and
   `SCOPE_WORKFLOW_IMAGE`, then run `deploy/northflank/create-run-job.sh`.
4. Copy the returned job `data.id` into the worker as `NORTHFLANK_JOB_ID`.
5. Give the Northflank token only the project job read/run/abort permissions documented by
   Northflank. Store it in the backend secret manager, never in a workflow or repository.

## Worker configuration

Set the following on the existing worker service:

```text
SCOPE_CLOUD_RUNS_ENABLED=true
SCOPE_PUBLIC_API_URL=https://api.your-scope-domain.example
SCOPE_RUNTIME_VERSION=<release identifier>
SCOPE_CLOUD_RUNS_MAX_CONCURRENCY=20
NORTHFLANK_API_TOKEN=<secret>
NORTHFLANK_PROJECT_ID=<project id>
NORTHFLANK_JOB_ID=<manual job id>
NORTHFLANK_DEPLOYMENT_PLAN=<4-vCPU/16-GiB plan id>
NORTHFLANK_REGISTRY_CREDENTIALS_ID=<optional saved credential id for private images>
```

Run exactly one worker service replica. Concurrency is handled inside that process; multiple
replicas would each enforce their own limit and make the spend ceiling approximate instead of
strict.

The API and worker keep using the existing S3-compatible bucket credentials. Run caches are stored
under `run-caches/v1/` and transferred with 15-minute signed URLs. Set a bucket lifecycle rule to
expire that prefix after 30 days; source objects use their existing lifecycle and encryption path.

## Rollout and rollback

The `m0020_cloud_execution` maintenance migration intentionally purges pre-alpha run history and
drops all self-hosted runner tables. Confirm there are no active attempts, take a database snapshot,
then use the existing maintenance deployment workflow. This migration is not reversible in place.

Start with `SCOPE_CLOUD_RUNS_MAX_CONCURRENCY=1`, run one manual canary, cancel one canary, and run a
cache workflow twice. Verify a Northflank external run ID appears in the run detail and the second
cache preparation is warm. Raise the limit only after those checks pass. To stop new spend without
rolling back schema or code, set the concurrency to one and disable cloud runs on the worker; queued
runs remain durable until execution is re-enabled.

## Cost controls

Northflank bills the selected compute plan for job runtime. The hard concurrency setting is the
primary spend ceiling. The workflow timeout, 24-hour Northflank deadline, 20 GiB ephemeral disk,
10 GiB per-cache archive limit, and 30-day cache lifecycle bound the other cost dimensions. Alert on
daily Northflank usage, aborted-run count, dispatch rejection count, and S3 bytes under
`run-caches/v1/`.
