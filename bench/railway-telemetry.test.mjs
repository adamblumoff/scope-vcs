import assert from 'node:assert/strict';
import test from 'node:test';

import {
  capacityRejectionFields, compactionFields, numericFields, objectStoreFields,
  gitOperationFields, gitSegmentTelemetryFields, isPushPersistenceMessage, pushPersistenceFields, railwayMetricArgs, stripAnsi,
  summarizeCapacityRejections, summarizeCompactions, summarizeGitOperations,
  summarizeGitSegmentTelemetry, summarizeMaterializations, summarizeObjectStore, summarizePushPersistence,
  summarizeSnapshots,
} from './railway-telemetry.mjs';

test('Git segment telemetry parses ingest, restore, pressure, and cleanup fields', () => {
  const ingest = gitSegmentTelemetryFields('Git segment ingest telemetry phase=local_write repository_id=owner/repo segment_id=seg-1 success=true duration_us=1200 bytes=1048576 blocked_us=30 active_ingests=2 buffered_bytes=2097152 disk_free_bytes=8589934592 ledger_uploading=2 ledger_ready=1 ledger_published=9 orphan_count=0');
  const restore = gitSegmentTelemetryFields('Git segment restore telemetry phase=frame_decrypt repository_id=owner/repo segment_id=seg-1 success=false duration_us=900 bytes=524288');
  assert.deepEqual(ingest, {
    kind: 'ingest', phase: 'local_write', repositoryId: 'owner/repo', segmentId: 'seg-1', success: true,
    duration_us: 1200, bytes: 1048576, blocked_us: 30, active_ingests: 2,
    buffered_bytes: 2097152, disk_free_bytes: 8589934592, ledger_uploading: 2,
    ledger_ready: 1, ledger_published: 9, orphan_count: 0,
  });
  assert.equal(restore.kind, 'restore');
  assert.equal(restore.phase, 'frame_decrypt');
  assert.equal(restore.success, false);
  assert.equal(gitSegmentTelemetryFields('ordinary Git log'), null);

  const summary = summarizeGitSegmentTelemetry([
    ingest,
    { ...ingest, phase: 'tee_remote_blocked', duration_us: 1500, blocked_us: 70, active_ingests: 3, buffered_bytes: 3145728, disk_free_bytes: 7516192768, ledger_published: 10, orphan_count: 1 },
    restore,
  ]);
  assert.deepEqual(summary.phases['ingest/local_write'], {
    count: 1,
    failures: 0,
    durationUs: { minimum: 1200, p50: 1200, p95: 1200, p99: 1200, maximum: 1200 },
    blockedUs: { minimum: 30, p50: 30, p95: 30, p99: 30, maximum: 30 },
    totalBytes: 1048576,
  });
  assert.equal(summary.phases['restore/frame_decrypt'].failures, 1);
  assert.deepEqual(summary.activeIngests, { minimum: 2, maximum: 3, last: 3 });
  assert.deepEqual(summary.diskFreeBytes, { minimum: 7516192768, maximum: 8589934592, last: 7516192768 });
  assert.deepEqual(summary.ledgerPublished, { minimum: 9, maximum: 10, last: 10 });
  assert.deepEqual(summary.orphanCount, { minimum: 0, maximum: 1, last: 1 });
});

test('resource metrics use the exact requested run window', () => {
  assert.deepEqual(
    railwayMetricArgs('scope-api', 'staging', '2026-08-22T20:00:00Z', '2026-08-22T20:05:00Z'),
    [
      'metrics', '--service', 'scope-api', '--environment', 'staging',
      '--since', '2026-08-22T20:00:00Z', '--raw', '--cpu', '--memory', '--json',
      '--until', '2026-08-22T20:05:00Z',
    ],
  );
});

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
  const parsed = pushPersistenceFields('Git push persistence timing repository_id=repo-1 protocol="transaction" config_changed=false changed_file_count=441 live_file_count=500 lock_wait_us=7 domain_apply_us=5 history_rows_us=6 serialized_us=11 body_us=13 commit_us=17 total_us=48');
  assert.deepEqual(parsed, {
    repositoryId: 'repo-1', protocol: 'transaction', configChanged: false,
    changed_file_count: 441, live_file_count: 500, lock_wait_us: 7, domain_apply_us: 5,
    history_rows_us: 6, serialized_us: 11, body_us: 13, commit_us: 17, total_us: 48,
  });
  const summary = summarizePushPersistence([parsed, { ...parsed, lock_wait_us: 9, total_us: 60 }]).transaction;
  assert.equal(summary.count, 2);
  assert.equal(summary.configChanges, 0);
  assert.deepEqual(summary.changedFileCount, { minimum: 441, maximum: 441, last: 441 });
  assert.deepEqual(summary.lockWaitUs, { minimum: 7, p50: 7, p95: 9, p99: 9, maximum: 9 });
  assert.deepEqual(summary.domainApplyUs, { minimum: 5, p50: 5, p95: 5, p99: 5, maximum: 5 });
  assert.deepEqual(summary.historyRowsUs, { minimum: 6, p50: 6, p95: 6, p99: 6, maximum: 6 });
  assert.deepEqual(summary.totalUs, { minimum: 48, p50: 48, p95: 60, p99: 60, maximum: 60 });
  assert.equal(summary.cloneUs, null);
  assert.equal(isPushPersistenceMessage('repository mutation persistence timing protocol=aggregate-mutation'), true);
  assert.equal(isPushPersistenceMessage('ordinary log'), false);
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

test('Git content telemetry counts validated cat-file output bytes', () => {
  const read = gitOperationFields('http_request{request_id=abc replica_id=replica-a}: Git content read completed operation=cat_file duration_ms=3 git_oid=deadbeef expected_size_bytes=1024 actual_size_bytes=1024 success=true');
  assert.equal(read.expected_size_bytes, 1024);
  assert.equal(read.actual_size_bytes, 1024);
  assert.equal(read.size_bytes, 1024);
  assert.equal(summarizeGitOperations([read]).cat_file.totalBytes, 1024);
});
