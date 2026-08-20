#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { mkdir, mkdtemp, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';
import { pathToFileURL } from 'node:url';

import { writeLinearHistoryStream } from './git-history.mjs';
import { normalizedRates, percentile, round, sampleStats } from './metrics.mjs';

// Black-box benchmark: no production-only hooks and never a production target.
const DEFAULT_STAGES = [1, 2, 4, 8];
const DEFAULT_WORKLOADS = [
  'warm-fetch', 'incremental-fetch', 'full-clone', 'code-read', 'repo-read',
  'projection-read', 'tree-read', 'blob-read', 'history-read', 'cold-churn', 'mixed', 'consistency',
];
const SUPPORTED_WORKLOADS = new Set(DEFAULT_WORKLOADS);
const activeCommands = new Set();

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) await main();

async function main() {
  const config = configuration();
  assertSafeTarget(config.api);
  await ready(config);
  const runRoot = join(config.outputRoot, new Date().toISOString().replaceAll(':', '-'));
  await mkdir(runRoot, { recursive: true });
  const fixtures = [];
  const clients = [];
  const report = {
    version: 3, generatedAt: new Date().toISOString(), apiUrl: config.api, config: publicConfig(config),
    workloads: [], faultHook: null,
    cleanup: { attemptedRepositories: 0, attemptedClients: 0, failed: [] },
  };
  let interrupted = false;
  const interrupt = () => {
    interrupted = true;
    for (const child of activeCommands) killCommandTree(child);
  };
  process.once('SIGINT', interrupt);
  process.once('SIGTERM', interrupt);
  try {
    console.log(`target: ${config.api} · node scale: ${config.nodeScaleLabel} · protocol: ${config.protocolLabel}`);
    console.log(config.rates ? `rates: ${config.rates.join(', ')}/s` : `concurrency: ${config.stages.join(', ')}`);
    console.log('seeding public repositories for read and mixed workloads...');
    const readFixtures = [];
    for (const depth of config.historyDepths) {
      const fixture = await seedRepository(config, runRoot, `history-${depth}`, config.readBytes, depth);
      fixtures.push(fixture);
      readFixtures.push(fixture);
      console.log(`  history fixture: ${depth} commits`);
    }
    const churnFixtures = [];
    for (let index = 0; index < config.churnRepos; index += 1) {
      const fixture = await seedRepository(config, runRoot, `churn-${index + 1}`, config.readBytes, config.historyDepths[0]);
      fixtures.push(fixture);
      churnFixtures.push(fixture);
    }
    const mixedFixtures = [];
    for (let index = 0; index < config.mixedRepos; index += 1) {
      const fixture = await seedRepository(
        config,
        runRoot,
        `mixed-${index + 1}`,
        config.readBytes,
        config.historyDepths[0],
        config.writeDeltaBytes[index % config.writeDeltaBytes.length],
      );
      fixtures.push(fixture);
      mixedFixtures.push(fixture);
    }
    const fetchClients = await createFetchClients(config, runRoot, mixedFixtures);
    clients.push(...fetchClients);
    if (config.faultHookUrl) report.faultHook = await invokeFaultHook(config);
    const context = { config, runRoot, readFixtures, churnFixtures, mixedFixtures, fetchClients, interrupted: () => interrupted };
    for (const name of config.workloads) {
      if (interrupted) break;
      console.log(`\n${name}`);
      report.workloads.push(await runStaircase(name, context));
      await persist(report, runRoot);
    }
  } finally {
    console.log('\ncleaning up benchmark fixtures...');
    report.cleanup.attemptedRepositories = fixtures.length;
    report.cleanup.attemptedClients = clients.length;
    await Promise.all(clients.map((client) => rm(client.parent, { recursive: true, force: true })));
    await Promise.all(fixtures.map(async (fixture) => {
      try { await deleteRepository(config, fixture); }
      catch (error) { report.cleanup.failed.push({ repo: `${fixture.owner}/${fixture.repo}`, error: message(error) }); }
      await rm(fixture.dir, { recursive: true, force: true });
    }));
    report.completedAt = new Date().toISOString();
    await persist(report, runRoot);
  }
  const paths = await persist(report, runRoot);
  console.log(`results: ${paths.json}\nsummary: ${paths.markdown}`);
  if (interrupted || report.workloads.some((workload) => workload.status === 'failed')) process.exitCode = 1;
}

function configuration() {
  const api = required('SCOPE_BENCH_API_URL').replace(/\/$/, '');
  const token = required('SCOPE_BENCH_AUTH_TOKEN');
  const stages = parseStages(process.env.SCOPE_LOAD_STAGES || DEFAULT_STAGES.join(','));
  const writeDeltaBytes = parseByteSizes(process.env.SCOPE_LOAD_WRITE_DELTA_BYTES || String(64 * 1024));
  return {
    api, token, stages,
    rates: process.env.SCOPE_LOAD_RATES ? parseRates(process.env.SCOPE_LOAD_RATES) : null,
    workloads: list('SCOPE_LOAD_WORKLOADS', DEFAULT_WORKLOADS),
    stageSeconds: positiveNumber('SCOPE_LOAD_STAGE_SECONDS', 120),
    confirmSeconds: nonNegativeNumber('SCOPE_LOAD_CONFIRM_SECONDS', 300),
    timeoutMs: positiveNumber('SCOPE_LOAD_TIMEOUT_MS', 90_000),
    cleanupTimeoutMs: positiveNumber('SCOPE_LOAD_CLEANUP_TIMEOUT_MS', 10_000),
    maxInFlight: positiveInteger('SCOPE_LOAD_MAX_IN_FLIGHT', 128),
    churnRepos: positiveInteger('SCOPE_LOAD_CHURN_REPOS', 16),
    mixedRepos: Math.max(writeDeltaBytes.length, positiveInteger('SCOPE_LOAD_MIXED_REPOS', Math.max(8, ...stages))),
    readBytes: positiveInteger('SCOPE_LOAD_READ_BYTES', 384 * 1024),
    writeDeltaBytes,
    historyDepths: parseStages(process.env.SCOPE_LOAD_HISTORY_DEPTHS || '1,16,64'),
    mixedWritePercent: boundedNumber('SCOPE_LOAD_MIXED_WRITE_PERCENT', 20, 0, 100),
    apiPermitLimits: {
      receivePack: positiveInteger('SCOPE_BENCH_RECEIVE_PACK_CONCURRENCY', 4),
      uploadPack: positiveInteger('SCOPE_BENCH_UPLOAD_PACK_CONCURRENCY', 8),
      projectionBuild: positiveInteger('SCOPE_BENCH_PROJECTION_BUILD_CONCURRENCY', 2),
      objectStore: positiveInteger('SCOPE_BENCH_OBJECT_STORE_CONCURRENCY', 16),
    },
    nodeScaleLabel: nonEmpty('SCOPE_LOAD_NODE_SCALE_LABEL', 'unspecified'),
    protocolLabel: nonEmpty('SCOPE_LOAD_PROTOCOL_LABEL', 'current'),
    runLabel: nonEmpty('SCOPE_BENCH_RUN_LABEL', 'unlabeled'),
    faultHookUrl: process.env.SCOPE_LOAD_FAULT_HOOK_URL?.trim() || null,
    consistencyTimeoutMs: positiveNumber('SCOPE_LOAD_CONSISTENCY_TIMEOUT_MS', 30_000),
    consistencyPollMs: positiveNumber('SCOPE_LOAD_CONSISTENCY_POLL_MS', 50),
    outputRoot: resolve(process.env.SCOPE_BENCH_OUTPUT_DIR || '.tmp/bench/railway-load'),
  };
}

function publicConfig(config) {
  const { token: _token, ...safe } = config;
  return safe;
}

export function assertSafeTarget(api) {
  const url = new URL(api);
  const local = ['localhost', '127.0.0.1', '::1'].includes(url.hostname);
  const loadtest = url.hostname.toLowerCase().split('.').some((label) => label.split('-').includes('loadtest'));
  if (!local && !loadtest) throw new Error(`refusing non-loadtest target: ${url.hostname}`);
}

export function parseStages(value) {
  const stages = [...new Set(value.split(',').map((entry) => Number.parseInt(entry.trim(), 10)))];
  if (!stages.length || stages.some((stage) => !Number.isInteger(stage) || stage < 1)) throw new Error('value must be a comma-separated list of positive integers');
  return stages.sort((left, right) => left - right);
}

export function parseRates(value) {
  const rates = [...new Set(value.split(',').map((entry) => Number(entry.trim())))];
  if (!rates.length || rates.some((rate) => !Number.isFinite(rate) || rate <= 0)) throw new Error('value must be a comma-separated list of positive numbers');
  return rates.sort((left, right) => left - right);
}

async function runStaircase(name, context) {
  const stages = [];
  let baselineP95 = null;
  let lastHealthy = null;
  let firstUnhealthy = null;
  const operation = operationFor(name, context);
  for (const target of context.config.rates || context.config.stages) {
    if (context.interrupted()) break;
    const stage = context.config.rates
      ? await timedRateStage(name, target, context.config.stageSeconds, operation, context)
      : await timedConcurrencyStage(name, target, context.config.stageSeconds, operation, context);
    baselineP95 ??= stage.stats.p95Ms;
    stage.gate = evaluateStage(stage, baselineP95);
    stages.push(stage);
    printStage(stage);
    if (!stage.gate.healthy) { firstUnhealthy = stage; break; }
    lastHealthy = stage;
  }
  const confirmations = [];
  let confirmedHealthy = null;
  if (!context.interrupted() && lastHealthy && context.config.confirmSeconds > 0) {
    for (const candidate of stages.filter((stage) => stage.gate.healthy).reverse()) {
      const target = candidate.targetRate ?? candidate.concurrency;
      console.log(`  confirming ${candidate.targetRate ? 'r' : 'c'}=${target} for ${context.config.confirmSeconds}s...`);
      const stage = context.config.rates
        ? await timedRateStage(name, target, context.config.confirmSeconds, operation, context)
        : await timedConcurrencyStage(name, target, context.config.confirmSeconds, operation, context);
      stage.gate = evaluateStage(stage, baselineP95);
      confirmations.push(stage);
      printStage(stage);
      if (stage.gate.healthy) { confirmedHealthy = stage; break; }
    }
  }
  const healthy = context.config.confirmSeconds > 0 ? confirmedHealthy : lastHealthy;
  return {
    name, status: healthy ? 'measured' : 'failed', nodeScaleLabel: context.config.nodeScaleLabel,
    protocolLabel: context.config.protocolLabel,
    baselineP95Ms: baselineP95, stages, confirmations,
    firstUnhealthy: firstUnhealthy ? firstUnhealthy.targetRate ?? firstUnhealthy.concurrency : null,
    lastHealthyConcurrency: healthy?.concurrency ?? null,
    lastHealthyTargetRate: healthy?.targetRate ?? null,
    lastHealthyThroughputPerSecond: healthy?.throughputPerSecond ?? null,
    safeMaxPerSecond: healthy ? round(healthy.throughputPerSecond * 0.7) : null,
  };
}

function operationFor(name, context) {
  const read = rotating(context.readFixtures);
  const churn = rotating(context.churnFixtures);
  const pairs = pooled(context.fetchClients);
  const writes = pooled(context.mixedFixtures);
  if (name === 'warm-fetch') return (_worker, _iteration, scheduledAt) => withResource(pairs, (pair) => gitFetch(context.config, pair, scheduledAt));
  if (name === 'incremental-fetch') return (_worker, iteration, scheduledAt) => withResource(writes, async (fixture) => {
    const pair = context.fetchClients.find((client) => client.fixture === fixture);
    if (!pair) throw new Error(`missing fetch client for ${fixture.owner}/${fixture.repo}`);
    const update = await updateAndPush(context.config, fixture, iteration, scheduledAt);
    return update.ok ? gitFetch(context.config, pair, scheduledAt) : update;
  });
  if (name === 'full-clone') return (_worker, _iteration, scheduledAt) => clone(context.config, context.runRoot, read(), scheduledAt);
  if (name === 'code-read') return async (_worker, _iteration, scheduledAt) => {
    const fixture = read();
    const result = await command(context.config, ['git', 'ls-remote', '--refs', fixture.publicRemote], undefined, undefined, scheduledAt);
    return {
      ...result,
      logicalBytes: result.bytes,
      historyDepth: fixture.historyDepth,
      byteSource: 'git-command-output',
    };
  };
  if (name === 'repo-read') return (_worker, _iteration, scheduledAt) => { const fixture = read(); return apiRead(context.config, repoPath(fixture), scheduledAt, fixture); };
  if (name === 'projection-read') return (_worker, _iteration, scheduledAt) => { const fixture = read(); return apiRead(context.config, `${repoPath(fixture)}/projection-preview?audience=public&source=live`, scheduledAt, fixture); };
  if (name === 'tree-read') return (_worker, _iteration, scheduledAt) => { const fixture = read(); return apiRead(context.config, `${repoPath(fixture)}/files`, scheduledAt, fixture); };
  if (name === 'blob-read') return (_worker, _iteration, scheduledAt) => { const fixture = read(); return apiRead(context.config, `${repoPath(fixture)}/files/content?path=load-update.txt`, scheduledAt, fixture); };
  if (name === 'history-read') return (_worker, _iteration, scheduledAt) => { const fixture = read(); return apiRead(context.config, `${repoPath(fixture)}/commits?audience=public`, scheduledAt, fixture); };
  if (name === 'cold-churn') return (_worker, _iteration, scheduledAt) => clone(context.config, context.runRoot, churn(), scheduledAt);
  if (name === 'mixed') {
    let index = 0;
    return (_worker, iteration, scheduledAt) => {
      const write = chooseWrite(index++, context.config.mixedWritePercent);
      if (write) return withResource(writes, (fixture) => updateAndPush(context.config, fixture, iteration, scheduledAt));
      const fixture = read();
      return apiRead(context.config, `${repoPath(fixture)}/files/content?path=load-update.txt`, scheduledAt, fixture);
    };
  }
  if (name === 'consistency') return (_worker, iteration, scheduledAt) => withResource(writes, (fixture) => writeThenVerify(context.config, fixture, iteration, scheduledAt));
  throw new Error(`unsupported workload: ${name}`);
}

function rotating(items) {
  let index = 0;
  return () => items[index++ % items.length];
}

function pooled(items) {
  const available = [...items];
  const waiters = [];
  return {
    acquire: () => available.length ? Promise.resolve(available.shift()) : new Promise((resolveItem) => waiters.push(resolveItem)),
    release: (item) => {
      const waiter = waiters.shift();
      if (waiter) waiter(item);
      else available.push(item);
    },
  };
}

async function withResource(pool, operation) {
  const resource = await pool.acquire();
  try { return await operation(resource); } finally { pool.release(resource); }
}

export function chooseWrite(index, percent) { return (index * percent) % 100 < percent; }

async function timedConcurrencyStage(name, concurrency, seconds, operation, context) {
  const samples = [];
  const startedAt = new Date().toISOString();
  const started = performance.now();
  const deadline = started + seconds * 1000;
  await Promise.all(Array.from({ length: concurrency }, async (_, worker) => {
    let iteration = 0;
    while (performance.now() < deadline && !context.interrupted()) samples.push(await operation(worker, iteration++));
  }));
  return stageResult(name, concurrency, samples, (performance.now() - started) / 1000, startedAt, new Date().toISOString(), null, context.config.nodeScaleLabel);
}

async function timedRateStage(name, targetRate, seconds, operation, context) {
  const samples = [];
  const inFlight = new Set();
  const startedAt = new Date().toISOString();
  const started = performance.now();
  const deadline = started + seconds * 1000;
  const intervalMs = 1000 / targetRate;
  let iteration = 0;
  for (let scheduledAt = started; scheduledAt < deadline && !context.interrupted(); scheduledAt += intervalMs) {
    await sleep(Math.max(0, scheduledAt - performance.now()));
    if (inFlight.size >= context.config.maxInFlight) {
      samples.push(sample(false, scheduledAt, 'client-saturated', 0, 'load generator in-flight limit reached'));
      continue;
    }
    const scheduleDelayMs = Math.max(0, performance.now() - scheduledAt);
    const pending = Promise.resolve(operation(0, iteration++, scheduledAt)).then((result) => {
      result.scheduleDelayMs = scheduleDelayMs;
      samples.push(result);
    }).finally(() => inFlight.delete(pending));
    inFlight.add(pending);
  }
  await Promise.all(inFlight);
  return stageResult(name, null, samples, (performance.now() - started) / 1000, startedAt, new Date().toISOString(), targetRate, context.config.nodeScaleLabel);
}

export function stageResult(name, concurrency, samples, elapsedSeconds, startedAt, completedAt, targetRate = null, nodeScaleLabel = 'unspecified') {
  const result = stats(samples);
  const rates = normalizedRates(samples, elapsedSeconds);
  return {
    name, concurrency, targetRate, nodeScaleLabel, startedAt, completedAt,
    elapsedSeconds: round(elapsedSeconds),
    throughputPerSecond: rates.operationsPerSecond,
    bytesPerSecond: round(result.bytes / elapsedSeconds),
    logicalBytesPerSecond: rates.logicalMiBPerSecond === null ? null : round(rates.logicalMiBPerSecond * 1024 * 1024),
    normalized: rates,
    errorRate: round((result.count - result.ok) / Math.max(1, result.count)),
    stats: result,
    historySizeSlope: historySizeSlope(samples),
    writeSizeSlope: writeSizeSlope(samples),
    consistency: consistencyStats(samples),
    failureBreakdown: failureBreakdown(samples),
    capacityRejections: capacityRejectionBreakdown(samples),
    failures: samples.filter((entry) => !entry.ok).slice(0, 5),
  };
}

export function evaluateStage(stage, baselineP95Ms) {
  const reasons = [];
  if (stage.errorRate > 0.01) reasons.push(`error rate ${(stage.errorRate * 100).toFixed(2)}% > 1%`);
  if (baselineP95Ms > 0 && stage.stats.p95Ms > baselineP95Ms * 2) reasons.push(`p95 ${stage.stats.p95Ms}ms > 2x baseline ${baselineP95Ms}ms`);
  if (stage.targetRate && stage.stats.scheduleDelayP95Ms > 1000 / stage.targetRate) reasons.push(`load generator p95 schedule delay ${stage.stats.scheduleDelayP95Ms}ms exceeded one arrival interval`);
  return { healthy: reasons.length === 0, reasons };
}

export function failureBreakdown(samples) {
  const failures = new Map();
  for (const entry of samples.filter(({ ok }) => !ok)) {
    const http = (entry.error || '').match(/(?:HTTP |error: )(\d{3})/i)?.[1];
    const kind = http ? `http-${http}` : entry.status === 'client-saturated' ? entry.status : /SIGKILL|signal/i.test(entry.error || '') ? 'killed' : 'other';
    failures.set(kind, (failures.get(kind) || 0) + 1);
  }
  return Object.fromEntries([...failures.entries()].sort());
}

export function capacityRejectionBreakdown(samples) {
  const rejections = new Map();
  for (const entry of samples.filter(({ ok }) => !ok)) {
    const match = (entry.error || '').match(/(Git receive-pack|Git upload-pack|Git projection build|object store (?:read|write|delete)) capacity is exhausted/i);
    if (!match) continue;
    rejections.set(match[1], (rejections.get(match[1]) || 0) + 1);
  }
  return Object.fromEntries([...rejections.entries()].sort());
}

function printStage(stage) {
  const gate = stage.gate.healthy ? 'healthy' : `stop: ${stage.gate.reasons.join('; ')}`;
  const target = stage.targetRate ? `r=${stage.targetRate}/s` : `c=${stage.concurrency}`;
  console.log(`  ${target} · ${stage.throughputPerSecond}/s · completion p95 ${stage.stats.p95Ms}ms · TTFB p95 ${stage.stats.ttfbP95Ms}ms · ${(stage.errorRate * 100).toFixed(2)}% errors · ${gate}`);
}

async function seedRepository(config, runRoot, label, bytes, historyDepth, writeDeltaBytes = 0, attempt = 1) {
  const created = await apiJson(config, '/v1/repos', { method: 'POST', body: { name: `loadtest-${label}-${Date.now()}-${randomBytes(3).toString('hex')}`, file_default_visibility: 'Public' } });
  const issuedPushToken = created.init.token ?? created.init.push_token;
  const fixture = {
    owner: created.repo.owner_handle, repo: created.repo.name,
    remote: new URL(new URL(created.init.git_remote_url).pathname, `${config.api}/`).toString(),
    publicRemote: `${config.api}/git/public/${encodeURIComponent(created.repo.owner_handle)}/${encodeURIComponent(created.repo.name)}`,
    branch: created.init.push_branch || 'main', pushToken: issuedPushToken?.secret,
    dir: await mkdtemp(join(runRoot, `${label}-`)), historyDepth, logicalBytes: bytes, writeDeltaBytes, update: 0,
  };
  try {
    for (const args of [['init'], ['symbolic-ref', 'HEAD', 'refs/heads/main'], ['config', 'user.email', 'loadtest@scope.local'], ['config', 'user.name', 'Scope Load Test']]) await checkedGit(config, args, fixture.dir);
    await mkdir(join(fixture.dir, '.scope'), { recursive: true });
    await writeFile(join(fixture.dir, '.scope', 'RULES.md'), '');
    await writePayload(fixture.dir, bytes);
    await writeFile(join(fixture.dir, 'load-update.txt'), 'seed\n');
    await checkedGit(config, ['add', '--all'], fixture.dir);
    await checkedGit(config, ['commit', '-m', `Seed ${label}`], fixture.dir);
    if (historyDepth > 1) await addFixtureHistory(config, fixture.dir, historyDepth - 1);
    const initialPush = await pushCurrentHead(config, fixture);
    if (!initialPush.ok) throw new Error(initialPush.error || 'initial fixture push failed');
    fixture.pushToken = config.token;
    return fixture;
  } catch (error) {
    await deleteRepository(config, fixture).catch(() => {});
    await rm(fixture.dir, { recursive: true, force: true });
    if (attempt < 3 && /(?:HTTP |error: )5\d\d/i.test(message(error))) {
      console.warn(`  retrying ${label} fixture after transient service failure (${attempt}/3)`);
      await sleep(250 * attempt);
      return seedRepository(config, runRoot, label, bytes, historyDepth, writeDeltaBytes, attempt + 1);
    }
    throw error;
  }
}

async function addFixtureHistory(config, repo, count) {
  const streamPath = join(repo, '.git', 'scope-bench-history.fast-import');
  const base = await gitOutput(config, ['rev-parse', 'HEAD'], repo);
  await writeLinearHistoryStream(streamPath, base, count);
  try {
    await checkedGit(config, ['fast-import', '--quiet'], repo, streamPath);
  } finally {
    await rm(streamPath, { force: true });
  }
}

async function createFetchClients(config, runRoot, fixtures) {
  const count = Math.max(config.mixedRepos, ...config.stages);
  const clients = [];
  for (let index = 0; index < count; index += 1) {
    const fixture = fixtures[index % fixtures.length];
    const parent = await mkdtemp(join(runRoot, 'fetch-client-'));
    const dir = join(parent, 'repo.git');
    const initial = await command(config, ['git', 'clone', '--quiet', '--bare', fixture.publicRemote, dir]);
    if (!initial.ok) {
      await rm(parent, { recursive: true, force: true });
      throw new Error(initial.error || 'initial fetch client clone failed');
    }
    clients.push({ fixture, dir, parent });
  }
  return clients;
}

async function writePayload(directory, bytes) {
  const payloadDir = join(directory, 'fixture');
  await mkdir(payloadDir, { recursive: true });
  let remaining = bytes;
  let index = 0;
  while (remaining > 0) {
    const size = Math.min(remaining, 256 * 1024);
    await writeFile(join(payloadDir, `${String(index++).padStart(4, '0')}.bin`), randomBytes(size));
    remaining -= size;
  }
}

async function updateAndPush(config, fixture, iteration, scheduledAt = performance.now()) {
  try {
    fixture.update += 1;
    const marker = `${fixture.update}:${iteration}:${Date.now()}`;
    await writeFile(join(fixture.dir, 'load-update.txt'), `${marker}\n`);
    if (fixture.writeDeltaBytes > 0) await writeFile(join(fixture.dir, 'load-delta.bin'), randomBytes(fixture.writeDeltaBytes));
    await checkedGit(config, ['add', 'load-update.txt', ...(fixture.writeDeltaBytes > 0 ? ['load-delta.bin'] : [])], fixture.dir);
    await checkedGit(config, ['commit', '-m', `Load update ${fixture.update}`], fixture.dir);
    const pushed = await pushCurrentHead(config, fixture, scheduledAt);
    return {
      ...pushed,
      historyDepth: fixture.historyDepth,
      logicalBytes: fixture.writeDeltaBytes + Buffer.byteLength(`${marker}\n`),
      writeDeltaBytes: fixture.writeDeltaBytes,
      marker,
    };
  } catch (error) {
    return { ...sample(false, scheduledAt, null, 0, message(error)), historyDepth: fixture.historyDepth };
  }
}

async function pushCurrentHead(config, fixture, started = performance.now()) {
  const repoConfig = await apiJson(config, `${repoPath(fixture)}/config`);
  const head = await gitOutput(config, ['rev-parse', 'HEAD'], fixture.dir);
  const intent = await apiJson(config, `${repoPath(fixture)}/push-intents`, { method: 'POST', body: { head_oid: head, base_config_hash: repoConfig.config_hash, config: repoConfig.config } });
  return command(config, ['git', '-c', 'push.recurseSubmodules=no', 'push', fixture.remote, `HEAD:${fixture.branch}`], fixture.dir, authEnvironment(fixture.remote, fixture.pushToken, intent.token), started);
}

async function gitFetch(config, pair, scheduledAt = performance.now()) {
  const before = await gitObjectBytes(config, pair.dir);
  const result = await command(config, ['git', 'fetch', '--quiet', 'origin'], pair.dir, undefined, scheduledAt);
  const after = result.ok ? await gitObjectBytes(config, pair.dir) : before;
  const bytes = Math.max(0, after - before);
  return { ...result, bytes, logicalBytes: bytes, historyDepth: pair.fixture.historyDepth, byteSource: 'local-git-object-delta' };
}

async function gitObjectBytes(config, directory) {
  const output = await gitOutput(config, ['count-objects', '-v'], directory);
  const fields = Object.fromEntries(output.split('\n').map((line) => line.split(':').map((part) => part.trim())));
  return ((Number(fields.size) || 0) + (Number(fields['size-pack']) || 0)) * 1024;
}

async function clone(config, runRoot, fixture, scheduledAt = performance.now()) {
  const parent = await mkdtemp(join(runRoot, 'clone-'));
  const destination = join(parent, 'repo.git');
  try {
    const result = await command(config, ['git', 'clone', '--quiet', '--bare', fixture.publicRemote, destination], undefined, undefined, scheduledAt);
    return {
      ...result,
      bytes: result.ok ? await directoryBytes(destination) : 0,
      logicalBytes: fixture.logicalBytes,
      historyDepth: fixture.historyDepth,
      byteSource: 'cloned-git-directory',
    };
  } finally { await rm(parent, { recursive: true, force: true }); }
}

async function directoryBytes(path) {
  let total = 0;
  const pending = [path];
  while (pending.length) {
    const current = pending.pop();
    const info = await stat(current);
    if (info.isDirectory()) for (const entry of await readdir(current)) pending.push(join(current, entry));
    else total += info.size;
  }
  return total;
}

async function writeThenVerify(config, fixture, iteration, scheduledAt = performance.now()) {
  const pushed = await updateAndPush(config, fixture, iteration, scheduledAt);
  if (!pushed.ok) return pushed;
  const visibilityStarted = performance.now();
  const deadline = visibilityStarted + config.consistencyTimeoutMs;
  const reads = [];
  let read;
  while (performance.now() < deadline) {
    const remainingMs = Math.max(1, deadline - performance.now());
    read = await apiRead(config, `${repoPath(fixture)}/files/content?path=load-update.txt`, performance.now(), fixture, true, Math.min(config.timeoutMs, remainingMs));
    reads.push(read);
    if (read.ok && read.json?.content?.text === `${pushed.marker}\n`) break;
    if (!read.ok && read.status !== 503) break;
    await sleep(config.consistencyPollMs);
  }
  const observed = read?.json?.content?.text;
  const ok = read?.ok === true && observed === `${pushed.marker}\n`;
  const completed = performance.now();
  const durationMs = completed - scheduledAt;
  const visibilityMs = completed - visibilityStarted;
  const readBytes = reads.reduce((total, sample) => total + sample.bytes, 0);
  return {
    ...read,
    ok,
    durationMs,
    completionMs: durationMs,
    ttfbMs: durationMs,
    bytes: pushed.bytes + readBytes,
    logicalBytes: pushed.logicalBytes + (read?.logicalBytes || 0),
    error: ok ? null : read?.ok ? `read-after-write mismatch: expected ${pushed.marker}` : read?.error || `marker was not visible within ${config.consistencyTimeoutMs}ms`,
    marker: pushed.marker,
    visibilityMs: round(visibilityMs),
    visibilityAttempts: reads.length,
    transientReadErrors: reads.filter(({ ok: readOk }) => !readOk).length,
    staleReads: reads.filter((sample) => sample.ok && sample.json?.content?.text !== `${pushed.marker}\n`).length,
  };
}

async function apiRead(config, path, started = performance.now(), fixture = null, parseJson = false, timeoutMs = config.timeoutMs) {
  try {
    const response = await fetch(`${config.api}${path}`, { headers: { accept: 'application/json' }, signal: AbortSignal.timeout(abortTimeoutMs(timeoutMs)) });
    const ttfbMs = performance.now() - started;
    const body = await response.text();
    const detail = body.trim().replace(/\s+/g, ' ').slice(0, 500);
    const result = sample(response.ok, started, response.status, Buffer.byteLength(body), response.ok ? null : `GET ${path}: HTTP ${response.status}${detail ? ` ${detail}` : ''}`, ttfbMs);
    result.logicalBytes = result.bytes;
    if (fixture) result.historyDepth = fixture.historyDepth;
    if (parseJson && response.ok) result.json = JSON.parse(body);
    return result;
  } catch (error) {
    return { ...sample(false, started, null, 0, message(error)), ...(fixture ? { historyDepth: fixture.historyDepth } : {}) };
  }
}

export function abortTimeoutMs(value) {
  return Math.max(1, Math.ceil(value));
}

async function deleteRepository(config, fixture) {
  await apiJson(config, repoPath(fixture), { method: 'DELETE', timeoutMs: config.cleanupTimeoutMs });
}

function repoPath(fixture) { return `/v1/repos/${encodeURIComponent(fixture.owner)}/${encodeURIComponent(fixture.repo)}`; }

async function checkedGit(config, args, cwd, stdinPath = null) {
  const result = await command(config, ['git', ...args], cwd, undefined, undefined, false, stdinPath);
  if (!result.ok) throw new Error(result.error || `git ${args.join(' ')} failed`);
}

async function gitOutput(config, args, cwd) {
  const result = await command(config, ['git', ...args], cwd, undefined, undefined, true);
  if (!result.ok) throw new Error(result.error || `git ${args.join(' ')} failed`);
  return result.output.trim();
}

function command(config, [program, ...args], cwd, extraEnv, started = performance.now(), capture = false, stdinPath = null) {
  return new Promise((resolveSample) => {
    const child = spawn(program, args, { cwd, detached: process.platform !== 'win32', env: { ...process.env, ...extraEnv, GIT_TERMINAL_PROMPT: '0' }, stdio: [stdinPath ? 'pipe' : 'ignore', 'pipe', 'pipe'] });
    activeCommands.add(child);
    const chunks = [];
    let firstByteMs = null;
    const collect = (chunk) => { firstByteMs ??= performance.now() - started; chunks.push(chunk); };
    child.stdout.on('data', collect);
    child.stderr.on('data', collect);
    if (stdinPath) createReadStream(stdinPath).pipe(child.stdin);
    const timer = setTimeout(() => killCommandTree(child), config.timeoutMs);
    let settled = false;
    const finish = (value) => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        activeCommands.delete(child);
        resolveSample(value);
      }
    };
    child.on('error', (error) => finish(sample(false, started, null, 0, error.message, firstByteMs)));
    child.on('close', (code, signal) => {
      const output = Buffer.concat(chunks).toString('utf8');
      const result = sample(code === 0, started, code, Buffer.byteLength(output), code === 0 ? null : output.slice(-1200) || String(signal), firstByteMs);
      if (capture) result.output = output;
      finish(result);
    });
  });
}

function killCommandTree(child) {
  if (!child.pid) return;
  try {
    if (process.platform === 'win32') child.kill('SIGKILL');
    else process.kill(-child.pid, 'SIGKILL');
  } catch (error) {
    if (error?.code !== 'ESRCH') child.kill('SIGKILL');
  }
}

async function apiJson(config, path, options = {}) {
  const headers = apiHeaders(config.token);
  if (options.body) headers['content-type'] = 'application/json';
  const response = await fetch(`${config.api}${path}`, { method: options.method || 'GET', headers, body: options.body ? JSON.stringify(options.body) : undefined, signal: AbortSignal.timeout(options.timeoutMs || config.timeoutMs) });
  const body = await response.text();
  if (!response.ok) throw new Error(`${options.method || 'GET'} ${path}: HTTP ${response.status} ${body.slice(0, 500)}`);
  return body ? JSON.parse(body) : null;
}

export function apiHeaders(token) {
  return { accept: 'application/json', authorization: `Bearer ${token}`, 'x-scope-cli-protocol': '1' };
}

function authEnvironment(destination, secret, pushIntent) {
  return {
    GIT_CONFIG_COUNT: '2',
    GIT_CONFIG_KEY_0: `http.${destination}.extraHeader`,
    GIT_CONFIG_VALUE_0: `Authorization: Bearer ${secret}`,
    GIT_CONFIG_KEY_1: `http.${destination}.extraHeader`,
    GIT_CONFIG_VALUE_1: `X-Scope-Push-Intent: ${pushIntent}`,
  };
}

async function ready(config) {
  const response = await fetch(`${config.api}/readyz`, { signal: AbortSignal.timeout(config.timeoutMs) });
  if (!response.ok) throw new Error(`API is not ready at ${config.api}: HTTP ${response.status}`);
}

async function invokeFaultHook(config) {
  const started = performance.now();
  try {
    const response = await fetch(config.faultHookUrl, { method: 'POST', signal: AbortSignal.timeout(config.timeoutMs) });
    return { url: config.faultHookUrl, ok: response.ok, status: response.status, durationMs: round(performance.now() - started) };
  } catch (error) {
    return { url: config.faultHookUrl, ok: false, error: message(error), durationMs: round(performance.now() - started) };
  }
}

function sample(ok, started, status, bytes, error, ttfbMs = null) {
  const durationMs = performance.now() - started;
  return { ok, durationMs, completionMs: durationMs, ttfbMs: ttfbMs ?? durationMs, status, bytes, error };
}

export function stats(values) { return sampleStats(values); }

export function historySizeSlope(samples) {
  const groups = new Map();
  for (const entry of samples) {
    if (!Number.isInteger(entry.historyDepth)) continue;
    const values = groups.get(entry.historyDepth) || [];
    values.push(entry);
    groups.set(entry.historyDepth, values);
  }
  const points = [...groups.entries()].sort(([left], [right]) => left - right).map(([historyDepth, values]) => ({ historyDepth, ...stats(values) }));
  if (points.length < 2) return { points, p95MsPerCommit: null };
  const first = points[0];
  const last = points.at(-1);
  return { points, p95MsPerCommit: round((last.p95Ms - first.p95Ms) / (last.historyDepth - first.historyDepth)) };
}

export function writeSizeSlope(samples) {
  const groups = new Map();
  for (const entry of samples) {
    if (!Number.isSafeInteger(entry.writeDeltaBytes)) continue;
    const values = groups.get(entry.writeDeltaBytes) || [];
    values.push(entry);
    groups.set(entry.writeDeltaBytes, values);
  }
  const points = [...groups.entries()]
    .sort(([left], [right]) => left - right)
    .map(([writeDeltaBytes, values]) => ({ writeDeltaBytes, ...stats(values) }));
  if (points.length < 2) return { points, p95MsPerMiB: null };
  const first = points[0];
  const last = points.at(-1);
  const deltaMiB = (last.writeDeltaBytes - first.writeDeltaBytes) / 1024 / 1024;
  return { points, p95MsPerMiB: deltaMiB > 0 ? round((last.p95Ms - first.p95Ms) / deltaMiB) : null };
}

export function consistencyStats(samples) {
  const values = samples.filter(({ visibilityMs }) => Number.isFinite(visibilityMs));
  if (!values.length) return null;
  const visibility = values.map(({ visibilityMs }) => visibilityMs);
  return {
    count: values.length,
    visibilityP50Ms: percentile(visibility, 0.5),
    visibilityP95Ms: percentile(visibility, 0.95),
    visibilityP99Ms: percentile(visibility, 0.99),
    attempts: values.reduce((total, sample) => total + sample.visibilityAttempts, 0),
    transientReadErrors: values.reduce((total, sample) => total + sample.transientReadErrors, 0),
    staleReads: values.reduce((total, sample) => total + sample.staleReads, 0),
  };
}

async function persist(report, output) {
  const json = join(output, 'results.json');
  const markdownPath = join(output, 'summary.md');
  await writeFile(json, `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(markdownPath, markdown(report));
  return { json, markdown: markdownPath };
}

function markdown(report) {
  const rows = report.workloads.map((workload) => {
    const stage = workload.confirmations.at(-1) || workload.stages.at(-1);
    return `| ${workload.name} | ${workload.status} | ${workload.lastHealthyThroughputPerSecond ?? '—'} | ${stage?.normalized.logicalMiBPerSecond ?? '—'} | ${stage?.stats.p95Ms ?? '—'} | ${stage?.stats.ttfbP95Ms ?? '—'} | ${stage?.stats.p99Ms ?? '—'} | ${stage?.normalized.observedMiBPerSecond ?? '—'} |`;
  }).join('\n');
  const permits = report.config.apiPermitLimits;
  const rejectionRows = report.workloads.flatMap((workload) => workload.stages.flatMap((stage) =>
    Object.entries(stage.capacityRejections || {}).map(([operation, count]) =>
      `| ${workload.name} | ${stage.concurrency ?? stage.targetRate} | ${operation} | ${count} |`,
    ))).join('\n') || '| none | n/a | none | 0 |';
  return `# Scope Railway Git storage load test\n\nGenerated: ${report.generatedAt}\n\nTarget: ${report.apiUrl}\n\nNode scale label: ${report.config.nodeScaleLabel}\n\nProtocol label: ${report.config.protocolLabel}\n\nAPI permit labels per process: receive-pack ${permits.receivePack}, upload-pack ${permits.uploadPack}, projection build ${permits.projectionBuild}, object store ${permits.objectStore}.\n\n| Workload | Status | Operations/s | Logical MiB/s | Completion p95 ms | TTFB p95 ms | Completion p99 ms | Observed MiB/s |\n|---|---|---:|---:|---:|---:|---:|---:|\n${rows}\n\n## Capacity rejections\n\n| Workload | Concurrency or rate | Operation | Count |\n|---|---:|---|---:|\n${rejectionRows}\n\nLogical MiB/s uses fixture payload sizes for writes and clones, and response or received-object bytes for reads. Observed MiB/s uses response bytes or local Git object deltas. Neither is a wire-level counter. TTFB for JSON reads is time to response headers. Quiet Git commands commonly emit no output, so their completion time is reported as TTFB. Compare node-scale and protocol labels only when repository fixture sizes, stage controls, and Railway deployment shape are identical.\n`;
}

function required(name) { const value = process.env[name]?.trim(); if (!value) throw new Error(`${name} is required`); return value; }
function list(name, fallback) {
  const values = (process.env[name] || fallback.join(',')).split(',').map((value) => value.trim()).filter(Boolean);
  const unknown = values.filter((value) => !SUPPORTED_WORKLOADS.has(value));
  if (unknown.length) throw new Error(`${name} has unsupported workloads: ${unknown.join(', ')}`);
  return [...new Set(values)];
}
export function parseByteSizes(value) {
  const entries = value.split(',').map((entry) => entry.trim());
  const sizes = [...new Set(entries.map((entry) => /^\d+$/.test(entry) ? Number(entry) : Number.NaN))];
  if (!sizes.length || sizes.some((size) => !Number.isSafeInteger(size) || size < 0)) throw new Error('SCOPE_LOAD_WRITE_DELTA_BYTES must be a comma-separated list of non-negative byte counts');
  return sizes.sort((left, right) => left - right);
}
function positiveInteger(name, fallback) { const value = Number.parseInt(process.env[name] || String(fallback), 10); if (!Number.isInteger(value) || value < 1) throw new Error(`${name} must be a positive integer`); return value; }
function positiveNumber(name, fallback) { const value = Number(process.env[name] || String(fallback)); if (!Number.isFinite(value) || value <= 0) throw new Error(`${name} must be positive`); return value; }
function nonNegativeNumber(name, fallback) { const value = Number(process.env[name] || String(fallback)); if (!Number.isFinite(value) || value < 0) throw new Error(`${name} must be non-negative`); return value; }
function boundedNumber(name, fallback, minimum, maximum) { const value = Number(process.env[name] || String(fallback)); if (!Number.isFinite(value) || value < minimum || value > maximum) throw new Error(`${name} must be between ${minimum} and ${maximum}`); return value; }
function nonEmpty(name, fallback) { return process.env[name]?.trim() || fallback; }
function sleep(milliseconds) { return new Promise((resolveSleep) => setTimeout(resolveSleep, milliseconds)); }
function message(error) { return error instanceof Error ? error.message : String(error); }
