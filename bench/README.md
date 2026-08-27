# Git storage benchmarks

These benchmarks answer two different questions:

1. What can Git and the attached disk do without Scope, Postgres, HTTP, or object storage?
2. How much of that ceiling does a deployed Scope environment deliver for reads, writes, and mixed traffic?

Keep both results. A fast component result with a slow black-box result points at Scope or its dependencies. If the component result is already slow, another service will not fix it.

All fixtures and reports live under the ignored `.tmp/bench/` directory. The scripts remove fixture repositories and local clients after each run. They never add benchmark-only production endpoints.

## 1. Measure local Git and disk

`git-physics.mjs` creates disposable repositories and measures:

- object enumeration and hashing;
- pack creation and index-pack ingestion;
- a no-local bare clone;
- blob reads and full integrity checks;
- wall and CPU time, peak RSS, page faults, context switches, and GNU time file-system operation counts;
- input MiB/s, CPU milliseconds per MiB, and output-to-input byte ratio.

Run the small physical smoke test:

```bash
node bench/git-physics.mjs
```

The default smoke profile tests 1 MiB of compressible and random content with eight commits. The standard profile is large enough to expose compression and history costs:

```bash
SCOPE_PHYSICS_PROFILE=standard node bench/git-physics.mjs
```

The full profile includes 1 GiB, 10 GiB, 1,000-commit, and 100,000-commit boundaries. It needs substantial temporary disk and can run for hours:

```bash
SCOPE_PHYSICS_PROFILE=full \
SCOPE_PHYSICS_SAMPLES=3 \
SCOPE_PHYSICS_EVICT_BYTES=16GiB \
node bench/git-physics.mjs
```

`SCOPE_PHYSICS_CASES` overrides a profile with comma-separated `bytes:commits:content` cases. Content is `random` or `compressible`:

```bash
SCOPE_PHYSICS_CASES=1MiB:1000:random,1GiB:1000:random \
SCOPE_PHYSICS_OPERATIONS=pack,index,clone,blob-read \
node bench/git-physics.mjs
```

The first unpressured sample is labeled `first-touch`, not cold. The kernel may still have cached data. `SCOPE_PHYSICS_EVICT_BYTES` creates and reads a pressure file before every measured operation for a repeatable evicted-cache comparison without root access. Set it above the machine's available memory, while leaving enough free disk for the largest fixture.

## 2. Measure Scope end to end

`railway-load.mjs` runs against an isolated Railway load-test environment or localhost. It refuses any non-local hostname without a `loadtest` label. Fixture creation may retry a transient 5xx twice. Ordinary measured operations are never retried. The consistency workload polls a retryable projection rebuild on purpose and reports visibility p50/p95/p99, poll attempts, and transient read errors.

The suite covers:

- `warm-fetch`, `incremental-fetch`, `full-clone`, and `cold-churn` Git reads;
- `code-read`, `repo-read`, `projection-read`, `tree-read`, `blob-read`, and `history-read` public reads;
- deterministic mixed reads and writes;
- a write followed by an exact marker read to check acknowledged-write consistency.

Run a short diagnostic first:

```bash
SCOPE_BENCH_API_URL=https://scope-api-loadtest.up.railway.app \
SCOPE_BENCH_AUTH_TOKEN=scope_cli_... \
SCOPE_LOAD_WORKLOADS=warm-fetch,blob-read,mixed,consistency \
SCOPE_LOAD_STAGES=1,2 \
SCOPE_LOAD_STAGE_SECONDS=30 \
SCOPE_LOAD_CONFIRM_SECONDS=0 \
SCOPE_LOAD_NODE_SCALE_LABEL=api-1-worker-1 \
SCOPE_LOAD_PROTOCOL_LABEL=current \
SCOPE_BENCH_RUN_LABEL=current-api1-2026-08-20 \
node bench/railway-load.mjs
```

The report labels the API's per-process permits. Defaults are 4 receive-pack operations, 8 upload-pack operations, 2 Git materializations, and 16 object-store operations. If the deployment overrides them, pass the matching `SCOPE_BENCH_RECEIVE_PACK_CONCURRENCY`, `SCOPE_BENCH_UPLOAD_PACK_CONCURRENCY`, `SCOPE_BENCH_GIT_MATERIALIZATION_CONCURRENCY`, and `SCOPE_BENCH_OBJECT_STORE_CONCURRENCY` values. A 429 at one of these limits is admission control, not a hardware ceiling.

To isolate replica-local Git cache effects, pass every load-test API endpoint and choose one routing policy. A push's config read, push intent, and Git transfer stay on the same selected endpoint.

```bash
SCOPE_BENCH_API_URL=https://scope-api-loadtest-a.up.railway.app \
SCOPE_BENCH_API_URLS=https://scope-api-loadtest-a.up.railway.app,https://scope-api-loadtest-b.up.railway.app,https://scope-api-loadtest-c.up.railway.app \
SCOPE_LOAD_ROUTING_MODE=repository-affine \
SCOPE_LOAD_TOPOLOGY_LABEL=three-node-affine \
SCOPE_LOAD_REPEAT_INDEX=1 \
SCOPE_LOAD_ROUTING_SEED=20260822 \
SCOPE_LOAD_WARMUP_SECONDS=30 \
SCOPE_LOAD_WARMUP_CONCURRENCY=4 \
SCOPE_LOAD_STAGES=1,4,8,16 \
node bench/railway-load.mjs
```

`single` always uses the first endpoint, `random` chooses a seeded endpoint for each logical operation, and `repository-affine` hashes `owner/repository` to one endpoint. Run three repeats of single-node, three-node random, and three-node affine in a shuffled order. Keep build SHA, service resources, region, Postgres, object storage, fixtures, warmup, and stage controls identical.

Then run the write-size matrix and longer staircase:

```bash
SCOPE_BENCH_API_URL=https://scope-api-loadtest.up.railway.app \
SCOPE_BENCH_AUTH_TOKEN=scope_cli_... \
SCOPE_LOAD_WRITE_DELTA_BYTES=4096,262144,8388608 \
SCOPE_LOAD_HISTORY_DEPTHS=1,1000,100000 \
SCOPE_LOAD_STAGES=1,2,4,8,16,32,64,128 \
SCOPE_LOAD_STAGE_SECONDS=120 \
SCOPE_LOAD_CONFIRM_SECONDS=1800 \
SCOPE_LOAD_NODE_SCALE_LABEL=api-1-worker-1 \
SCOPE_LOAD_PROTOCOL_LABEL=current \
SCOPE_BENCH_RUN_LABEL=current-api1-2026-08-20 \
node bench/railway-load.mjs
```

The Git segment streaming acceptance run measures 64, 128, and 256 MiB pushes separately at c1, c2, and c4. The runner divides each delta into 16 MiB files so no file reaches Scope's 25 MiB per-file limit. It writes each file through a 256 KiB buffer, which keeps fixture generation from allocating the full delta in Node.

Use a staging service whose hostname contains a `loadtest` label. The target guard will reject the ordinary staging and production hostnames.

```bash
export SCOPE_BENCH_API_URL=https://scope-api-loadtest-staging.up.railway.app
export SCOPE_BENCH_AUTH_TOKEN=scope_cli_...

for bytes in 67108864 134217728 268435456; do
  SCOPE_LOAD_WORKLOADS=mixed \
  SCOPE_LOAD_MIXED_WRITE_PERCENT=100 \
  SCOPE_LOAD_WRITE_DELTA_BYTES="$bytes" \
  SCOPE_LOAD_MIXED_REPOS=4 \
  SCOPE_LOAD_STAGES=1,2,4 \
  SCOPE_LOAD_STAGE_SECONDS=120 \
  SCOPE_LOAD_CONFIRM_SECONDS=0 \
  SCOPE_LOAD_TIMEOUT_MS=600000 \
  SCOPE_LOAD_PROTOCOL_LABEL=git-segment-v2 \
  SCOPE_LOAD_NODE_SCALE_LABEL=api-2-worker-1 \
  SCOPE_BENCH_RUN_LABEL="git-segment-v2-${bytes}" \
  node bench/railway-load.mjs
done
```

Keep the API and worker sizes, replica count, region, database, object-store bucket, and local volume size fixed across all three runs. Record the exact start and end timestamps for each run, then collect telemetry over those windows.

To measure landing-file write cost, run the mixed and consistency workloads with unrelated pushes plus 4 KiB and 1 MiB `README.html` updates. Record the pre-change push p95 and pass it to the post-change run; the runner marks a stage unhealthy if its measured push p95 regresses by more than 15%:

```bash
SCOPE_LOAD_WORKLOADS=mixed,consistency \
SCOPE_LOAD_WRITE_DELTA_BYTES=0 \
SCOPE_LOAD_LANDING_FILE_BYTES=0,4096,1048576 \
SCOPE_LOAD_PUSH_BASELINE_P95_MS=250 \
node bench/railway-load.mjs
```

`SCOPE_LOAD_LANDING_FILE_BYTES=0` means the push changes only an unrelated file. Positive values create and update an exact-size `README.html`. Reports group push latency by landing-file size and include its p95 milliseconds-per-MiB slope.

Use `SCOPE_LOAD_RATES=1,2,4` for an open-loop arrival-rate staircase. The runner stops above 1% errors, twice the first-stage p95, or one arrival interval of client scheduling delay. `safeMaxPerSecond` is 70% of the last confirmed healthy throughput. It is a test result, not a production capacity promise.

Reports contain operations/s, logical MiB/s, observed MiB/s, p50/p95/p99 completion and TTFB, error classes, history-size slope, write-size slope, and landing-file-size slope. API bytes are response bytes. Git bytes are local received-object deltas or cloned directory sizes. These are not wire-level counters.

### Protocol and topology tournament

Run the exact same suite against each behavior-equivalent deployment and label it:

```bash
SCOPE_LOAD_PROTOCOL_LABEL=current SCOPE_LOAD_NODE_SCALE_LABEL=api-1 ...
SCOPE_LOAD_PROTOCOL_LABEL=minimal-cas SCOPE_LOAD_NODE_SCALE_LABEL=api-1 ...
SCOPE_LOAD_PROTOCOL_LABEL=batched-wal SCOPE_LOAD_NODE_SCALE_LABEL=api-1 ...
```

Repeat the winner at one, two, and four nodes. Do not compare runs unless fixture sizes, stage controls, database and object-store class, region, and build are identical.

Run `railway-telemetry.mjs` for the same interval. Its report separates capacity rejections from scheduler outcomes and records peak CPU, RSS, cgroup PIDs, open file descriptors, and zombie children. Use those counters to tell a fixed permit ceiling from CPU, memory, process, or shared-service saturation.

The benchmark does not fake CAS or WAL behavior with a SQL-only microbenchmark. Such a test omits pack durability, object-store calls, serialization, recovery, and read visibility. It cannot select the production protocol honestly. Deploy callable production variants, run this black-box suite, then delete the losing variants.

## 3. Collect Railway evidence

Collect logs and Railway CPU and memory metrics over the matching load window:

```bash
SCOPE_RAILWAY_ENVIRONMENT=loadtest-capacity \
SCOPE_RAILWAY_SINCE=1h \
SCOPE_BENCH_RUN_LABEL=current-api1-2026-08-20 \
node bench/railway-telemetry.mjs
```

The telemetry report groups:

- process, thread, file-descriptor, child, zombie, and cgroup PID snapshots;
- compaction phase timings and outcomes;
- push persistence lock wait, serialization, transaction body, commit, and total time by protocol;
- object-store operation latency, failures, bytes, and MiB per second of summed service time;
- Git segment ingest phases, including local write and fsync, tee blocking, frame encryption, and multipart first-part, last-part, completion, and abort timings;
- Git segment restore phases;
- active ingests, buffered bytes, minimum local disk free bytes, upload-ledger state counts, and orphan counts;
- Railway CPU and memory samples plus capacity and spawn errors.

Object-store service metrics and Postgres server-side wait events still require their provider dashboards or exports. The application events show where Scope spent time, but they do not replace storage-side request, queue, throttling, or database WAL evidence.

## Tests

```bash
node --test bench/*.test.mjs
```
