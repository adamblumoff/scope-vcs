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
5. Give the Northflank token only `Project > Jobs > General > Read`, `Run job`, and `Update`.
   Northflank requires `Update` to abort an active run. Store the token in the backend secret
   manager, never in a workflow or repository.

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

Run cache metadata and storage are owned by the separate `scope-cache-service`. The API gives each
claimed attempt a dedicated Ed25519-signed grant scoped to its repository, workflow cache
identities, backend, and maximum 24-hour run lifetime. The runner exchanges that grant for
15-minute signed URLs and transfers bytes directly to the cache bucket; neither the API nor cache
service proxies archives.

Provision a cache bucket that is separate from the durable repository/source bucket. Confirm the
Northflank job region before choosing the first cache backend region. Configure the cache service:

```text
DATABASE_URL=<same Postgres cluster; cache-service DB role>
SCOPE_CACHE_BACKEND=<lowercase provider-region id>
SCOPE_CACHE_BUCKET_ENDPOINT=<S3-compatible endpoint>
SCOPE_CACHE_BUCKET_NAME=<dedicated cache bucket>
SCOPE_CACHE_BUCKET_REGION=<bucket region>
SCOPE_CACHE_BUCKET_ACCESS_KEY_ID=<cache-only credential>
SCOPE_CACHE_BUCKET_SECRET_ACCESS_KEY=<secret>
SCOPE_CACHE_BUCKET_FORCE_PATH_STYLE=<true only when required>
SCOPE_CACHE_GRANT_PUBLIC_KEY=<Ed25519 public PEM>
```

Configure the API with the matching control-plane values. The private signing key belongs only to
the API:

```text
SCOPE_CACHE_URL=https://<cache-service-host>
SCOPE_CACHE_BACKEND=<same provider-region id>
SCOPE_CACHE_GRANT_PRIVATE_KEY=<Ed25519 private PEM>
```

Cache objects are immutable and repository-scoped under
`repos/<repository-id>/objects/sha256/<digest>`. The service owns a 1 GiB object ceiling, a 5 GiB
per-repository LRU budget, seven-day sliding reference TTL, 30-minute upload leases, one-hour
deletion grace, and reconciliation. Do not add a bucket lifecycle policy that can delete referenced
objects independently of this metadata.

## Rollout and rollback

The `m0021_cache_service_cutover` maintenance migration creates the cache-service schema and drops
`scope_run_cache_objects`. It intentionally does not copy disposable pre-alpha cache state. Confirm
there are no active attempts, take a database snapshot, stop API/worker/cache-service writers, and
run the existing maintenance deployment workflow. After the new API, cache service, and runner
image are deployed, delete every legacy object under `run-caches/v1/`. This migration is not
reversible in place.

Start with `SCOPE_CLOUD_RUNS_MAX_CONCURRENCY=1`, run one manual canary, cancel one canary, and run a
cache workflow twice. Verify a Northflank external run ID appears in the run detail, the second cache
preparation is warm, and an unchanged third run sends zero PUT bytes. Temporarily deny the cache
service and bucket in separate canaries; both runs must finish cold rather than fail. Raise the limit
only after those checks pass. To stop new spend without rolling back schema or code, set concurrency
to one and disable cloud runs on the worker; queued runs remain durable until execution is
re-enabled.

## Cost controls

Northflank bills the selected compute plan for job runtime and runner egress. The hard concurrency
setting is the primary compute ceiling. The workflow timeout, 24-hour Northflank deadline, 20 GiB
ephemeral disk, 1 GiB object limit, 5 GiB repository budget, and seven-day TTL bound the other cost
dimensions. Alert on daily Northflank usage, cache GET/PUT bytes, unchanged saves, quota rejections,
expired leases, deletion retries, logical referenced bytes, and physical bucket bytes.
