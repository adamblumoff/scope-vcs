# Phase 0 Benchmarks

`bench/phase0.mjs` is the local baseline harness for the data-architecture
work. It intentionally lives outside `api/src`, `web/src`, and `cli/src`; the
first goal is to measure current hot paths without adding benchmark-only hooks
to production runtime code.

Run it through the local dev entrypoint:

```bash
./dev/scope-dev up
./dev/scope-dev bench
```

The harness uses the seeded local repositories created by `./dev/scope-dev up`:

- `public-demo` for public API reads and Git smart HTTP reads.
- `update-demo` for public reads against a repo with a staged update.
- owner-only/review routes are included when the harness finds your local Scope
  CLI session token, or when `SCOPE_BENCH_AUTH_TOKEN` is set explicitly.
- `git receive-pack` is measured with disposable generated repositories. Each
  push sample creates a throwaway repo through the API, pushes a generated Git
  fixture once, then deletes that repo.

Results are written under `.tmp/bench/phase0/<timestamp>/` as JSON and Markdown.
The directory is ignored by Git.

Configuration:

- `SCOPE_BENCH_API_URL` overrides the API origin. Default:
  `http://localhost:8080`.
- `SCOPE_BENCH_OWNER` overrides the seeded owner handle. By default the harness
  uses `SCOPE_DEV_USER_HANDLE`, then derives the same normalized handle from
  `SCOPE_DEV_USER_EMAIL` that the local API seed uses.
- `SCOPE_BENCH_WARMUP` controls warmup requests per case. Default: `2`.
- `SCOPE_BENCH_SAMPLES` controls sequential samples and burst rounds. Default:
  `12`.
- `SCOPE_BENCH_CONCURRENCY` controls burst fan-out. Default: `6`.
- `SCOPE_BENCH_TIMEOUT_MS` controls per-request timeout. Default: `15000`.
- `SCOPE_BENCH_PUSH_FILES` controls generated files per push fixture commit.
  Default: `4`.
- `SCOPE_BENCH_PUSH_COMMITS` controls commits per push fixture. Default: `2`.
- `SCOPE_BENCH_PUSH_BURST=1` enables burst concurrency for the mutating push
  fixture. It is off by default so normal local runs do not create many
  concurrent throwaway repos.
- `SCOPE_BENCH_AUTH_TOKEN` overrides auto-detected auth. By default the harness
  reuses the local `scope_cli_...` session token for the configured API URL when
  the CLI stores that token in the local session file.

The Phase 0 harness reports external latency, response size, status-code mix,
subprocess latency for `git ls-remote`, and first-push receive-pack latency.
Object-store byte counters, projection cache hit rates, and internal Git
subprocess counters are not observable from the public local API yet; those
remain explicit gaps for later instrumentation work.
