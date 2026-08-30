import assert from 'node:assert/strict';
import { mkdtemp, readdir, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import test from 'node:test';

import { createEndpointRouter, parseApiUrls } from './endpoint-routing.mjs';

import {
  abortTimeoutMs, apiHeaders, assertSafeTarget, capacityRejectionBreakdown, changedFileCountSlope, chooseWrite,
  consistencyStats, evaluateStage, failureBreakdown,
  historySizeSlope, landingFileSizeSlope, parseByteSizes, parseRates, parseStages, stageResult, stats,
  toggleBenchmarkVisibilityRule, WRITE_DELTA_FILE_BYTES, writeChunkedRandomPayload, writeSizeSlope,
} from './railway-load.mjs';
import { parseChangedFileCounts, writeChangedFiles } from './write-shape.mjs';

test('large write deltas use bounded files and buffers', async (context) => {
  const root = await mkdtemp(join(tmpdir(), 'scope-load-delta-'));
  context.after(() => rm(root, { recursive: true, force: true }));

  const paths = await writeChunkedRandomPayload(join(root, 'delta'), 35, 16, 5);
  assert.ok(WRITE_DELTA_FILE_BYTES < 25 * 1024 * 1024);
  assert.deepEqual(paths.map((path) => basename(path)), ['0000.bin', '0001.bin', '0002.bin']);
  assert.deepEqual(
    await Promise.all(paths.map(async (path) => (await stat(path)).size)),
    [16, 16, 3],
  );
  assert.equal((await readdir(join(root, 'delta'))).length, 3);
});

test('visibility polling rounds fractional AbortSignal timeouts up', () => {
  assert.equal(abortTimeoutMs(29_999.001), 30_000);
  assert.equal(abortTimeoutMs(0.01), 1);
});

test('load target guard accepts only local or loadtest hosts', () => {
  assert.doesNotThrow(() => assertSafeTarget('http://localhost:8080'));
  assert.doesNotThrow(() => assertSafeTarget('https://scope-api-loadtest.up.railway.app'));
  assert.throws(() => assertSafeTarget('https://scope-api-production.up.railway.app'), /refusing non-loadtest target/);
  assert.throws(() => assertSafeTarget('https://notloadtest.example.com'), /refusing non-loadtest target/);
});

test('numeric workload controls are unique and sorted', () => {
  assert.deepEqual(parseStages('4,1,2,4'), [1, 2, 4]);
  assert.deepEqual(parseRates('1,0.25,2,1'), [0.25, 1, 2]);
  assert.throws(() => parseStages('1,nope'), /positive integers/);
  assert.throws(() => parseRates('0,1'), /positive numbers/);
  assert.deepEqual(parseByteSizes('8388608,4096,262144,4096'), [4096, 262144, 8388608]);
  assert.throws(() => parseByteSizes('-1,1'), /non-negative byte counts/);
  assert.deepEqual(parseChangedFileCounts('500,1,100,1'), [1, 100, 500]);
  assert.throws(() => parseChangedFileCounts('-1,1'), /non-negative integers/);
});

test('changed-file fixtures preserve count and exact file size', async (context) => {
  const root = await mkdtemp(join(tmpdir(), 'scope-load-files-'));
  context.after(() => rm(root, { recursive: true, force: true }));

  await writeChangedFiles(root, 65, 32, 7);
  const files = await readdir(root);
  assert.equal(files.length, 65);
  assert.deepEqual(await Promise.all(files.map(async (file) => (await stat(join(root, file))).size)), Array(65).fill(32));
});

test('endpoint pools are normalized without duplicate primaries', () => {
  assert.deepEqual(
    parseApiUrls('https://api-1-loadtest.example.com/', 'https://api-1-loadtest.example.com, https://api-2-loadtest.example.com/'),
    ['https://api-1-loadtest.example.com', 'https://api-2-loadtest.example.com'],
  );
});

test('endpoint routing is reproducible and repository affinity is stable', () => {
  const urls = ['https://api-1-loadtest.example.com', 'https://api-2-loadtest.example.com', 'https://api-3-loadtest.example.com'];
  const randomA = createEndpointRouter(urls, 'random', 42);
  const randomB = createEndpointRouter(urls, 'random', 42);
  const keys = Array.from({ length: 12 }, (_, index) => `blob-read:4:${index}`);
  const sequenceA = keys.map((key) => randomA.choose('owner/repo', key));
  const sequenceB = [...keys].reverse().map((key) => randomB.choose('owner/repo', key)).reverse();
  assert.deepEqual(sequenceA, sequenceB);
  assert.ok(new Set(sequenceA).size > 1);

  const affine = createEndpointRouter(urls, 'repository-affine');
  const first = affine.choose('owner/repo');
  assert.equal(affine.choose('owner/repo'), first);
  assert.equal(affine.choose('owner/repo'), first);
  assert.equal(createEndpointRouter(urls, 'single').choose('another/repo'), urls[0]);
});

test('statistics report completion and TTFB p50 p95 p99 with bytes', () => {
  const values = [
    { ok: true, durationMs: 10, ttfbMs: 2, bytes: 1, logicalBytes: 10 },
    { ok: true, durationMs: 20, ttfbMs: 4, bytes: 2, logicalBytes: 20 },
    { ok: true, durationMs: 30, ttfbMs: 6, bytes: 3, logicalBytes: 30 },
    { ok: true, durationMs: 40, ttfbMs: 8, bytes: 4, logicalBytes: 40 },
  ];
  assert.deepEqual(stats(values), {
    count: 4, ok: 4, meanMs: 25,
    p50Ms: 20, p95Ms: 40, p99Ms: 40,
    ttfbP50Ms: 4, ttfbP95Ms: 8, ttfbP99Ms: 8,
    scheduleDelayP95Ms: 0, bytes: 10, logicalBytes: 100,
  });
});

test('history slope groups exact fixture depths and reports p95 growth', () => {
  const slope = historySizeSlope([
    { ok: true, durationMs: 10, ttfbMs: 1, bytes: 1, historyDepth: 1 },
    { ok: true, durationMs: 12, ttfbMs: 1, bytes: 1, historyDepth: 1 },
    { ok: true, durationMs: 40, ttfbMs: 2, bytes: 1, historyDepth: 8 },
    { ok: true, durationMs: 42, ttfbMs: 2, bytes: 1, historyDepth: 8 },
  ]);
  assert.deepEqual(slope, {
    points: [
      { historyDepth: 1, count: 2, ok: 2, meanMs: 11, p50Ms: 10, p95Ms: 12, p99Ms: 12, ttfbP50Ms: 1, ttfbP95Ms: 1, ttfbP99Ms: 1, scheduleDelayP95Ms: 0, bytes: 2, logicalBytes: 0 },
      { historyDepth: 8, count: 2, ok: 2, meanMs: 41, p50Ms: 40, p95Ms: 42, p99Ms: 42, ttfbP50Ms: 2, ttfbP95Ms: 2, ttfbP99Ms: 2, scheduleDelayP95Ms: 0, bytes: 2, logicalBytes: 0 },
    ],
    p95MsPerCommit: 4.29,
  });
});

test('mixed workload is deterministic at the requested 80/20 split', () => {
  assert.deepEqual(Array.from({ length: 10 }, (_, index) => chooseWrite(index, 20)), [true, false, false, false, false, true, false, false, false, false]);
});

test('stage results preserve node labels, byte rate, and errors', () => {
  const stage = stageResult('blob-read', 2, [
    { ok: true, durationMs: 10, ttfbMs: 2, bytes: 100, historyDepth: 1 },
    { ok: false, durationMs: 20, ttfbMs: 4, bytes: 0, historyDepth: 1, status: 503, error: 'HTTP 503' },
  ], 2, 'start', 'end', null, 'api=2');
  assert.equal(stage.nodeScaleLabel, 'api=2');
  assert.equal(stage.bytesPerSecond, 50);
  assert.equal(stage.errorRate, 0.5);
  assert.equal(stage.stats.p99Ms, 20);
  assert.equal(stage.normalized.operationsPerSecond, 0.5);
});

test('write-size slope separates payload sizes', () => {
  assert.deepEqual(writeSizeSlope([
    { ok: true, durationMs: 10, ttfbMs: 10, bytes: 1, logicalBytes: 4096, writeDeltaBytes: 4096 },
    { ok: true, durationMs: 30, ttfbMs: 30, bytes: 1, logicalBytes: 1048576, writeDeltaBytes: 1048576 },
  ]), {
    points: [
      { writeDeltaBytes: 4096, count: 1, ok: 1, meanMs: 10, p50Ms: 10, p95Ms: 10, p99Ms: 10, ttfbP50Ms: 10, ttfbP95Ms: 10, ttfbP99Ms: 10, scheduleDelayP95Ms: 0, bytes: 1, logicalBytes: 4096 },
      { writeDeltaBytes: 1048576, count: 1, ok: 1, meanMs: 30, p50Ms: 30, p95Ms: 30, p99Ms: 30, ttfbP50Ms: 30, ttfbP95Ms: 30, ttfbP99Ms: 30, scheduleDelayP95Ms: 0, bytes: 1, logicalBytes: 1048576 },
    ],
    p95MsPerMiB: 20.08,
  });
});

test('landing-file slope separates unrelated pushes from bounded README updates', () => {
  assert.deepEqual(landingFileSizeSlope([
    { ok: true, durationMs: 10, ttfbMs: 10, bytes: 1, logicalBytes: 1, landingFileBytes: 0 },
    { ok: true, durationMs: 20, ttfbMs: 20, bytes: 1, logicalBytes: 4096, landingFileBytes: 4096 },
    { ok: true, durationMs: 30, ttfbMs: 30, bytes: 1, logicalBytes: 1048576, landingFileBytes: 1048576 },
  ]), {
    points: [
      { landingFileBytes: 0, count: 1, ok: 1, meanMs: 10, p50Ms: 10, p95Ms: 10, p99Ms: 10, ttfbP50Ms: 10, ttfbP95Ms: 10, ttfbP99Ms: 10, scheduleDelayP95Ms: 0, bytes: 1, logicalBytes: 1 },
      { landingFileBytes: 4096, count: 1, ok: 1, meanMs: 20, p50Ms: 20, p95Ms: 20, p99Ms: 20, ttfbP50Ms: 20, ttfbP95Ms: 20, ttfbP99Ms: 20, scheduleDelayP95Ms: 0, bytes: 1, logicalBytes: 4096 },
      { landingFileBytes: 1048576, count: 1, ok: 1, meanMs: 30, p50Ms: 30, p95Ms: 30, p99Ms: 30, ttfbP50Ms: 30, ttfbP95Ms: 30, ttfbP99Ms: 30, scheduleDelayP95Ms: 0, bytes: 1, logicalBytes: 1048576 },
    ],
    p95MsPerMiB: 20,
  });
});

test('changed-file slope reports p95 cost per file', () => {
  assert.deepEqual(changedFileCountSlope([
    { ok: true, durationMs: 10, ttfbMs: 10, bytes: 1, changedFileCount: 1 },
    { ok: true, durationMs: 109, ttfbMs: 109, bytes: 1, changedFileCount: 100 },
  ]), {
    points: [
      { changedFileCount: 1, count: 1, ok: 1, meanMs: 10, p50Ms: 10, p95Ms: 10, p99Ms: 10, ttfbP50Ms: 10, ttfbP95Ms: 10, ttfbP99Ms: 10, scheduleDelayP95Ms: 0, bytes: 1, logicalBytes: 0 },
      { changedFileCount: 100, count: 1, ok: 1, meanMs: 109, p50Ms: 109, p95Ms: 109, p99Ms: 109, ttfbP50Ms: 109, ttfbP95Ms: 109, ttfbP99Ms: 109, scheduleDelayP95Ms: 0, bytes: 1, logicalBytes: 0 },
    ],
    p95MsPerFile: 1,
  });
});

test('aggregate benchmark pushes toggle an equivalent visibility rule', () => {
  const original = { visibility: { default: 'public', rules: [] } };
  const added = toggleBenchmarkVisibilityRule(original);
  assert.deepEqual(original.visibility.rules, []);
  assert.deepEqual(added.visibility.rules, [{ path: '/load-files/**', visibility: 'public' }]);
  assert.deepEqual(toggleBenchmarkVisibilityRule(added).visibility.rules, []);
});

test('consistency statistics expose projection convergence instead of hiding polls', () => {
  assert.deepEqual(consistencyStats([
    { visibilityMs: 20, visibilityAttempts: 1, transientReadErrors: 0, staleReads: 0 },
    { visibilityMs: 80, visibilityAttempts: 3, transientReadErrors: 2, staleReads: 1 },
  ]), {
    count: 2,
    visibilityP50Ms: 20,
    visibilityP95Ms: 80,
    visibilityP99Ms: 80,
    attempts: 4,
    transientReadErrors: 2,
    staleReads: 1,
  });
  assert.equal(consistencyStats([{ durationMs: 10 }]), null);
});

test('stage gate rejects errors and latency above twice baseline', () => {
  const stage = { name: 'blob-read', errorRate: 0.02, stats: { p95Ms: 210, scheduleDelayP95Ms: 0 }, landingFileSizeSlope: { points: [] } };
  assert.equal(evaluateStage(stage, 100).healthy, false);
  assert.equal(evaluateStage({ name: 'blob-read', errorRate: 0.01, stats: { p95Ms: 100, scheduleDelayP95Ms: 0 }, landingFileSizeSlope: { points: [] } }, 100).healthy, true);
});

test('push stage gate enforces the optional fifteen-percent regression budget', () => {
  const stage = {
    name: 'mixed', errorRate: 0, stats: { p95Ms: 100, scheduleDelayP95Ms: 0 },
    landingFileSizeSlope: { points: [{ landingFileBytes: 1048576, p95Ms: 116 }] },
  };
  assert.deepEqual(evaluateStage(stage, 100, 100), {
    healthy: false,
    reasons: ['push p95 116ms > 1.15x baseline 100ms'],
  });
  stage.landingFileSizeSlope.points[0].p95Ms = 115;
  assert.equal(evaluateStage(stage, 100, 100).healthy, true);
});

test('failure breakdown separates service responses from client saturation', () => {
  assert.deepEqual(failureBreakdown([
    { ok: false, status: null, error: 'POST failed: HTTP 503 unavailable' },
    { ok: false, status: 1, error: 'remote: error: 429' },
    { ok: false, status: 'client-saturated', error: 'limit reached' },
    { ok: true, status: 0, error: null },
  ]), { 'client-saturated': 1, 'http-429': 1, 'http-503': 1 });
});

test('capacity rejection breakdown names the exhausted permit', () => {
  assert.deepEqual(capacityRejectionBreakdown([
    { ok: false, error: 'fatal: remote error: Git materialization capacity is exhausted; retry later' },
    { ok: false, error: 'Git receive-pack capacity is exhausted; retry later' },
    { ok: false, error: 'HTTP 429' },
    { ok: true, error: 'Git receive-pack capacity is exhausted; retry later' },
  ]), {
    'Git materialization': 1,
    'Git receive-pack': 1,
  });
});

test('API mutations identify the supported CLI protocol', () => {
  assert.equal(apiHeaders('secret')['x-scope-cli-protocol'], '1');
});
