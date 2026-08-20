import assert from 'node:assert/strict';
import test from 'node:test';

import {
  configuration, parseBytes, parseCases, parseCountObjects, parseTimeReport, summarizeOperation,
} from './git-physics.mjs';

test('byte sizes use binary units and reject fractional bytes', () => {
  assert.equal(parseBytes('4KiB'), 4096);
  assert.equal(parseBytes('1.5MiB'), 1572864);
  assert.equal(parseBytes('1GiB'), 1073741824);
  assert.throws(() => parseBytes('1.1B'), /integer/);
  assert.throws(() => parseBytes('-1MiB'), /invalid/);
});

test('case parser preserves the physical dimensions', () => {
  assert.deepEqual(parseCases('4KiB:1:random,1MiB:1000:compressible'), [
    { label: '4KiB-1c-random', logicalBytes: 4096, commits: 1, content: 'random' },
    { label: '1MiB-1000c-compressible', logicalBytes: 1048576, commits: 1000, content: 'compressible' },
  ]);
  assert.throws(() => parseCases('1MiB:0:random'), /commit count/);
  assert.throws(() => parseCases('1MiB:1:mystery'), /content kind/);
});

test('smoke configuration stays small and accepts explicit matrix cases', () => {
  const config = configuration({
    SCOPE_PHYSICS_PROFILE: 'smoke',
    SCOPE_PHYSICS_CASES: '4KiB:2:random',
    SCOPE_PHYSICS_OPERATIONS: 'pack,index',
    SCOPE_PHYSICS_SAMPLES: '1',
    SCOPE_PHYSICS_FILE_BYTES: '4KiB',
    SCOPE_PHYSICS_EVICT_BYTES: '0',
    SCOPE_PHYSICS_TIMEOUT_MS: '1000',
    SCOPE_PHYSICS_OUTPUT_DIR: '.tmp/test-output',
  });
  assert.equal(config.cases[0].logicalBytes, 4096);
  assert.deepEqual(config.operations, ['pack', 'index']);
  assert.equal(config.samples, 1);
});

test('GNU time parser names CPU, page-cache, disk, and scheduler counters', () => {
  assert.deepEqual(parseTimeReport('__SCOPE_TIME__\t1.25\t0.50\t2.00\t8192\t3\t4\t5\t6\t7\t8\n'), {
    wallMs: 2000,
    userCpuMs: 1250,
    systemCpuMs: 500,
    cpuMs: 1750,
    maxRssKiB: 8192,
    fsInputOperations: 3,
    fsOutputOperations: 4,
    majorPageFaults: 5,
    minorPageFaults: 6,
    involuntaryContextSwitches: 7,
    voluntaryContextSwitches: 8,
  });
});

test('operation summary takes medians without hiding per-sample values', () => {
  const summary = summarizeOperation([
    { wallMs: 30, cpuMs: 20, maxRssKiB: 30, normalized: { inputMiBPerSecond: 2, cpuMsPerMiB: 3, outputToInputRatio: 1.2 } },
    { wallMs: 10, cpuMs: 5, maxRssKiB: 10, normalized: { inputMiBPerSecond: 1, cpuMsPerMiB: 2, outputToInputRatio: 1.1 } },
    { wallMs: 20, cpuMs: 10, maxRssKiB: 20, normalized: { inputMiBPerSecond: 3, cpuMsPerMiB: 4, outputToInputRatio: 1.3 } },
  ]);
  assert.deepEqual(summary, {
    samples: 3,
    medianWallMs: 20,
    medianCpuMs: 10,
    medianMaxRssKiB: 20,
    medianInputMiBPerSecond: 2,
    medianCpuMsPerMiB: 3,
    medianOutputToInputRatio: 1.2,
  });
});

test('operation summary averages the middle pair for an even sample count', () => {
  const normalized = { inputMiBPerSecond: 2, cpuMsPerMiB: 3, outputToInputRatio: 1 };
  assert.equal(summarizeOperation([
    { wallMs: 10, cpuMs: 5, maxRssKiB: 10, normalized },
    { wallMs: 30, cpuMs: 15, maxRssKiB: 30, normalized },
  ]).medianWallMs, 20);
});

test('count-objects parser converts Git KiB fields into bytes', () => {
  assert.deepEqual(parseCountObjects('count: 3\nsize: 7\nin-pack: 0\npacks: 2\nsize-pack: 11\n'), {
    count: 3,
    sizeBytes: 7168,
    packs: 2,
    packedBytes: 11264,
  });
});
