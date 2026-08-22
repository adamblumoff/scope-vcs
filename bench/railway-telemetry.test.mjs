import assert from 'node:assert/strict';
import test from 'node:test';

import {
  capacityRejectionFields, compactionFields, numericFields, objectStoreFields,
  gitOperationFields, pushPersistenceFields, stripAnsi, summarizeCapacityRejections,
  summarizeCompactions, summarizeGitOperations, summarizeMaterializations, summarizeObjectStore,
  summarizePushPersistence, summarizeSnapshots,
} from './railway-telemetry.mjs';

test('runtime snapshot parsing strips tracing colors and reads numeric fields', () => {
  const message = '\u001b[32mINFO\u001b[0m runtime process snapshot threads=31 cgroup_pids_current=48';
  assert.equal(stripAnsi(message), 'INFO runtime process snapshot threads=31 cgroup_pids_current=48');
  assert.deepEqual(numericFields(message, ['threads', 'cgroup_pids_current']), {
    threads: 31,
    cgroup_pids_current: 48,
  });
});

test('process summaries retain minimum, maximum, and final values', () => {
  assert.deepEqual(summarizeSnapshots([
    { threads: 27, cgroup_pids_current: 30 },
    { threads: 31, cgroup_pids_current: 52 },
    { threads: 29, cgroup_pids_current: 34 },
  ]), {
    threads: { minimum: 27, maximum: 31, last: 29 },
    cgroup_pids_current: { minimum: 30, maximum: 52, last: 34 },
  });
});

test('compaction outcomes are parsed without tracing quotes', () => {
  const parsed = compactionFields('Git compaction attempt completed outcome="stale" repo_id=owner/repo target_sequence=64 scheduler_attempts=1 scheduler_queue_delay_ms=250 total_ms=42');
  assert.deepEqual(
    parsed,
    {
      outcome: 'stale', repoId: 'owner/repo', target_sequence: 64, scheduler_attempts: 1,
      scheduler_queue_delay_ms: 250, total_ms: 42,
    },
  );
  assert.deepEqual(summarizeCompactions([parsed]), {
    count: 1,
    outcomes: { stale: 1 },
    queueDelayMs: { minimum: 250, p50: 250, p95: 250, p99: 250, maximum: 250 },
    attempts: { minimum: 1, p50: 1, p95: 1, p99: 1, maximum: 1 },
    totalMs: { minimum: 42, p50: 42, p95: 42, p99: 42, maximum: 42 },
  });
});

test('capacity rejection telemetry names each fixed API permit', () => {
  const events = [
    capacityRejectionFields('Git receive-pack capacity is exhausted; retry later'),
    capacityRejectionFields('fatal: remote error: Git materialization capacity is exhausted; retry later'),
    capacityRejectionFields('Git receive-pack capacity is exhausted; retry later'),
  ];
  assert.deepEqual(events, [
    { operation: 'Git receive-pack' },
    { operation: 'Git materialization' },
    { operation: 'Git receive-pack' },
  ]);
  assert.deepEqual(summarizeCapacityRejections(events), {
    'Git receive-pack': 2,
    'Git materialization': 1,
  });
  assert.deepEqual(
    capacityRejectionFields('runtime capacity permit rejected operation="Git upload-pack"'),
    { operation: 'Git upload-pack' },
  );
  assert.equal(capacityRejectionFields('ordinary infrastructure error'), null);
});

test('push persistence timings retain protocol and lock-held phases', () => {
  const parsed = pushPersistenceFields('Git push persistence timing repository_id=repo-1 protocol="transaction" lock_wait_us=7 serialized_us=11 body_us=13 commit_us=17 total_us=48');
  assert.deepEqual(parsed, {
    repositoryId: 'repo-1', protocol: 'transaction', lock_wait_us: 7, serialized_us: 11,
    body_us: 13, commit_us: 17, total_us: 48,
  });
  assert.deepEqual(summarizePushPersistence([parsed, { ...parsed, lock_wait_us: 9, total_us: 60 }]), {
    transaction: {
      count: 2,
      lockWaitUs: { minimum: 7, p50: 7, p95: 9, p99: 9, maximum: 9 },
      serializedUs: { minimum: 11, p50: 11, p95: 11, p99: 11, maximum: 11 },
      bodyUs: { minimum: 13, p50: 13, p95: 13, p99: 13, maximum: 13 },
      commitUs: { minimum: 17, p50: 17, p95: 17, p99: 17, maximum: 17 },
      totalUs: { minimum: 48, p50: 48, p95: 60, p99: 60, maximum: 60 },
    },
  });
});

test('object-store timings report failures and successful service-time byte rate', () => {
  const success = objectStoreFields('object store operation timing operation=put bytes=1048576 elapsed_us=500000 success=true');
  const failure = objectStoreFields('object store operation timing operation="put" bytes=0 elapsed_us=1000 success=false');
  assert.deepEqual(success, { operation: 'put', success: true, bytes: 1048576, elapsed_us: 500000 });
  assert.deepEqual(summarizeObjectStore([success, failure]), {
    put: {
      count: 2,
      failures: 1,
      elapsedUs: { minimum: 1000, p50: 1000, p95: 500000, p99: 500000, maximum: 500000 },
      totalBytes: 1048576,
      serviceTimeMiBPerSecond: 2,
    },
  });
});

test('Git restore telemetry stays joined to request, replica, cache, and frontier', () => {
  const materialization = gitOperationFields('http_request{request_id=abc replica_id=replica-a}: repository Git replica materialization completed repository_id=owner/repo cache_outcome=build materialization_path=restore elapsed_us=42000 requested_sequence=8 pack_span_count=3 total_pack_bytes=1048576 success=true');
  const download = gitOperationFields('http_request{request_id=abc replica_id=replica-a}: Git restore operation completed repository_id=owner/repo operation=object_retrieval duration_ms=12 size_bytes=1048576 span_index=1 span_count=3 first_sequence=1 last_sequence=4 geometric_tier=2 success=true');
  assert.deepEqual(materialization, {
    requestId: 'abc', replicaId: 'replica-a', repositoryId: 'owner/repo',
    operation: 'materialize_repository', cacheOutcome: 'build', materializationPath: 'restore',
    success: true, durationMs: 42, elapsed_us: 42000, requested_sequence: 8,
    pack_span_count: 3, total_pack_bytes: 1048576,
  });
  assert.equal(download.operation, 'object_retrieval');
  assert.equal(download.durationMs, 12);
  assert.equal(download.first_sequence, 1);
  assert.deepEqual(summarizeMaterializations([materialization, download]), {
    'build/restore': {
      count: 1,
      durationMs: { minimum: 42, p50: 42, p95: 42, p99: 42, maximum: 42 },
    },
  });
  assert.deepEqual(summarizeGitOperations([download]), {
    object_retrieval: {
      count: 1,
      failures: 0,
      durationMs: { minimum: 12, p50: 12, p95: 12, p99: 12, maximum: 12 },
      totalDurationMs: 12,
      totalBytes: 1048576,
    },
  });
});
