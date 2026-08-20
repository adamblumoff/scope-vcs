#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { createReadStream } from 'node:fs';
import {
  mkdir, mkdtemp, open, readFile, readdir, rm, stat, statfs, writeFile,
} from 'node:fs/promises';
import { cpus, freemem, platform, release, totalmem } from 'node:os';
import { join, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';
import { pathToFileURL } from 'node:url';

import { writeLinearHistoryStream } from './git-history.mjs';
import { bytesLabel, normalizeProcessMeasurement, round } from './metrics.mjs';

const DEFAULT_CASES = {
  smoke: '1MiB:8:compressible,1MiB:8:random',
  standard: '1MiB:1000:random,256MiB:1000:compressible,256MiB:1000:random',
  full: '1MiB:100000:compressible,1GiB:1000:compressible,1GiB:1000:random,10GiB:1000:random',
};
const OPERATIONS = new Set(['enumerate', 'pack', 'index', 'clone', 'blob-read', 'fsck']);
const TIME_FORMAT = '__SCOPE_TIME__\t%U\t%S\t%e\t%M\t%I\t%O\t%F\t%R\t%c\t%w';

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) await main();

async function main() {
  const config = configuration();
  const runId = new Date().toISOString().replaceAll(':', '-');
  const runRoot = join(config.outputRoot, runId);
  await mkdir(runRoot, { recursive: true });
  await assertTools(config);
  const report = {
    version: 1,
    kind: 'git-component-physics',
    generatedAt: new Date().toISOString(),
    config,
    host: await hostFacts(runRoot),
    cases: [],
    limitations: [
      'File-system counters come from GNU time and are operation counts, not bytes.',
      'First-touch is not guaranteed cold. Set SCOPE_PHYSICS_EVICT_BYTES above available memory pressure for an evicted-cache comparison.',
      'This isolates Git and local storage. PostgreSQL, object storage, HTTP, locks, and application scheduling belong to the black-box suite.',
      'Protocol comparisons require the same production behavior behind separately labeled deployments. A synthetic SQL-only CAS or WAL test would omit durability and recovery costs.',
    ],
  };
  try {
    for (const spec of config.cases) {
      console.log(`\n${spec.label}`);
      report.cases.push(await runCase(config, runRoot, spec));
      await persist(report, runRoot);
    }
    report.completedAt = new Date().toISOString();
    const paths = await persist(report, runRoot);
    console.log(`\nresults: ${paths.json}\nsummary: ${paths.markdown}`);
  } catch (error) {
    report.failedAt = new Date().toISOString();
    report.error = message(error);
    await persist(report, runRoot);
    throw error;
  }
}

export function configuration(env = process.env) {
  const profile = env.SCOPE_PHYSICS_PROFILE || 'smoke';
  if (!DEFAULT_CASES[profile]) throw new Error(`unknown SCOPE_PHYSICS_PROFILE: ${profile}`);
  return {
    profile,
    cases: parseCases(env.SCOPE_PHYSICS_CASES || DEFAULT_CASES[profile]),
    operations: parseOperations(env.SCOPE_PHYSICS_OPERATIONS || [...OPERATIONS].join(',')),
    samples: positiveInteger(env.SCOPE_PHYSICS_SAMPLES, profile === 'smoke' ? 2 : 3),
    fileBytes: parseBytes(env.SCOPE_PHYSICS_FILE_BYTES || '8MiB'),
    evictBytes: parseBytes(env.SCOPE_PHYSICS_EVICT_BYTES || '0'),
    timeoutMs: positiveInteger(env.SCOPE_PHYSICS_TIMEOUT_MS, 30 * 60 * 1000),
    runLabel: env.SCOPE_BENCH_RUN_LABEL?.trim() || 'unlabeled',
    outputRoot: resolve(env.SCOPE_PHYSICS_OUTPUT_DIR || '.tmp/bench/git-physics'),
  };
}

export function parseCases(value) {
  const labels = new Set();
  return value.split(',').map((entry) => {
    const [bytesValue, commitsValue, content] = entry.trim().split(':');
    const logicalBytes = parseBytes(bytesValue);
    const commits = Number.parseInt(commitsValue, 10);
    if (!Number.isInteger(commits) || commits < 1) throw new Error(`invalid commit count in physics case: ${entry}`);
    if (!['compressible', 'random'].includes(content)) throw new Error(`invalid content kind in physics case: ${entry}`);
    const label = `${bytesValue}-${commits}c-${content}`;
    if (labels.has(label)) throw new Error(`duplicate physics case: ${label}`);
    labels.add(label);
    return { label, logicalBytes, commits, content };
  });
}

export function parseBytes(value) {
  const match = String(value).trim().match(/^(\d+(?:\.\d+)?)\s*(B|KiB|MiB|GiB)?$/i);
  if (!match) throw new Error(`invalid byte size: ${value}`);
  const units = { b: 1, kib: 1024, mib: 1024 ** 2, gib: 1024 ** 3 };
  const bytes = Number(match[1]) * units[(match[2] || 'B').toLowerCase()];
  if (!Number.isSafeInteger(bytes) || bytes < 0) throw new Error(`byte size must resolve to a non-negative integer: ${value}`);
  return bytes;
}

export function parseTimeReport(value) {
  const line = value.trim().split('\n').find((entry) => entry.startsWith('__SCOPE_TIME__\t'));
  if (!line) throw new Error('GNU time did not produce a parseable measurement');
  const [, userSeconds, systemSeconds, wallSeconds, maxRssKiB, fsInputs, fsOutputs, majorFaults, minorFaults, involuntarySwitches, voluntarySwitches] = line.split('\t');
  const values = [userSeconds, systemSeconds, wallSeconds, maxRssKiB, fsInputs, fsOutputs, majorFaults, minorFaults, involuntarySwitches, voluntarySwitches].map(Number);
  if (values.some((entry) => !Number.isFinite(entry))) throw new Error('GNU time produced non-numeric measurement fields');
  return {
    wallMs: round(values[2] * 1000),
    userCpuMs: round(values[0] * 1000),
    systemCpuMs: round(values[1] * 1000),
    cpuMs: round((values[0] + values[1]) * 1000),
    maxRssKiB: values[3],
    fsInputOperations: values[4],
    fsOutputOperations: values[5],
    majorPageFaults: values[6],
    minorPageFaults: values[7],
    involuntaryContextSwitches: values[8],
    voluntaryContextSwitches: values[9],
  };
}

async function runCase(config, runRoot, spec) {
  const caseRoot = await mkdtemp(join(runRoot, 'case-'));
  const repo = join(caseRoot, 'source');
  try {
    console.log(`  building ${bytesLabel(spec.logicalBytes)}, ${spec.commits} commits, ${spec.content}`);
    const fixture = await createFixture(config, caseRoot, repo, spec);
    const measurements = [];
    let canonicalPack = null;
    for (const operation of config.operations) {
      const samples = [];
      for (let index = 0; index < config.samples; index += 1) {
        const cacheState = await prepareCache(config, caseRoot, index);
        const result = await runOperation(config, caseRoot, fixture, operation, index, canonicalPack);
        canonicalPack ||= result.packPath || null;
        const normalized = normalizeProcessMeasurement(result, result.inputBytes, result.outputBytes);
        const sample = { sample: index + 1, cacheState, ...result, normalized };
        delete sample.packPath;
        samples.push(sample);
        console.log(`  ${operation} ${index + 1}/${config.samples}: ${result.wallMs} ms, ${normalized.inputMiBPerSecond ?? 'n/a'} MiB/s, ${result.cpuMs} CPU ms`);
      }
      measurements.push({ operation, samples, summary: summarizeOperation(samples) });
    }
    return {
      spec,
      fixture: {
        repoBytesBeforePack: fixture.repoBytes,
        looseObjects: fixture.countObjects.count,
        looseObjectBytes: fixture.countObjects.sizeBytes,
        representativeBlobBytes: fixture.blobBytes,
      },
      measurements,
    };
  } finally {
    await rm(caseRoot, { recursive: true, force: true });
  }
}

async function createFixture(config, caseRoot, repo, spec) {
  await checked(config, 'git', ['init', '--quiet', repo]);
  await checked(config, 'git', ['-C', repo, 'symbolic-ref', 'HEAD', 'refs/heads/main']);
  await checked(config, 'git', ['-C', repo, 'config', 'user.email', 'physics@scope.local']);
  await checked(config, 'git', ['-C', repo, 'config', 'user.name', 'Scope Physics']);
  const payloadDir = join(repo, 'fixture');
  await mkdir(payloadDir, { recursive: true });
  let remaining = spec.logicalBytes;
  let index = 0;
  let representativeBlobBytes = 0;
  while (remaining > 0) {
    const size = Math.min(remaining, config.fileBytes);
    const contents = spec.content === 'random' ? randomBytes(size) : Buffer.alloc(size, 0x61);
    await writeFile(join(payloadDir, `${String(index++).padStart(6, '0')}.bin`), contents);
    representativeBlobBytes ||= size;
    remaining -= size;
  }
  if (spec.logicalBytes === 0) await writeFile(join(payloadDir, '000000.bin'), '');
  await checked(config, 'git', ['-C', repo, 'add', '--all']);
  await checked(config, 'git', ['-C', repo, 'commit', '--quiet', '-m', 'Base payload']);
  if (spec.commits > 1) await addHistory(config, caseRoot, repo, spec.commits - 1);
  const countObjects = parseCountObjects(await checkedOutput(config, 'git', ['-C', repo, 'count-objects', '-v']));
  return {
    repo,
    repoBytes: await directoryBytes(repo),
    countObjects,
    blobBytes: representativeBlobBytes,
    logicalBytes: spec.logicalBytes,
  };
}

async function addHistory(config, caseRoot, repo, count) {
  const streamPath = join(caseRoot, 'history.fast-import');
  const base = await checkedOutput(config, 'git', ['-C', repo, 'rev-parse', 'HEAD']);
  await writeLinearHistoryStream(streamPath, base, count);
  await checked(config, 'git', ['-C', repo, 'fast-import', '--quiet'], { stdinPath: streamPath });
  await rm(streamPath, { force: true });
}

async function runOperation(config, caseRoot, fixture, operation, sampleIndex, canonicalPack) {
  if (operation === 'enumerate') {
    return measured(config, caseRoot, 'git', ['-C', fixture.repo, 'rev-list', '--objects', '--all'], {
      inputBytes: fixture.logicalBytes,
      outputBytesFromStdout: true,
    });
  }
  if (operation === 'pack') {
    const outputDir = join(caseRoot, `pack-${sampleIndex}`);
    await mkdir(outputDir);
    const prefix = join(outputDir, 'objects');
    const result = await measured(config, caseRoot, 'git', ['-C', fixture.repo, 'pack-objects', prefix, '--all']);
    const files = await readdir(outputDir);
    const packName = files.find((name) => name.endsWith('.pack'));
    if (!packName) throw new Error('git pack-objects did not create a pack');
    const packPath = join(outputDir, packName);
    return { ...result, inputBytes: fixture.logicalBytes, outputBytes: await directoryBytes(outputDir), packPath };
  }
  if (operation === 'index') {
    if (!canonicalPack) throw new Error('index requires pack to run first');
    const destination = join(caseRoot, `index-${sampleIndex}.git`);
    await checked(config, 'git', ['init', '--quiet', '--bare', destination]);
    const inputBytes = (await stat(canonicalPack)).size;
    const result = await measured(config, caseRoot, 'git', ['-C', destination, 'index-pack', '--stdin'], { stdinPath: canonicalPack });
    return { ...result, inputBytes, outputBytes: await directoryBytes(destination) };
  }
  if (operation === 'clone') {
    const destination = join(caseRoot, `clone-${sampleIndex}.git`);
    const result = await measured(config, caseRoot, 'git', ['clone', '--quiet', '--bare', '--no-local', fixture.repo, destination]);
    return { ...result, inputBytes: fixture.logicalBytes, outputBytes: await directoryBytes(destination) };
  }
  if (operation === 'blob-read') {
    const result = await measured(config, caseRoot, 'git', ['-C', fixture.repo, 'cat-file', 'blob', 'HEAD:fixture/000000.bin']);
    return { ...result, inputBytes: fixture.blobBytes, outputBytes: result.stdoutBytes };
  }
  if (operation === 'fsck') {
    const result = await measured(config, caseRoot, 'git', ['-C', fixture.repo, 'fsck', '--full', '--strict']);
    return { ...result, inputBytes: fixture.repoBytes, outputBytes: result.stdoutBytes };
  }
  throw new Error(`unsupported physics operation: ${operation}`);
}

async function measured(config, workdir, program, args, options = {}) {
  const timePath = join(workdir, `time-${Date.now()}-${randomBytes(3).toString('hex')}.txt`);
  const started = performance.now();
  const execution = await execute('/usr/bin/time', ['-f', TIME_FORMAT, '-o', timePath, program, ...args], {
    cwd: workdir,
    stdinPath: options.stdinPath,
    timeoutMs: config.timeoutMs,
  });
  const timing = parseTimeReport(await readFile(timePath, 'utf8'));
  await rm(timePath, { force: true });
  if (execution.code !== 0) throw new Error(`${program} ${args.join(' ')} failed: ${execution.stderr.slice(-1000)}`);
  const measuredWallMs = round(performance.now() - started);
  return {
    ...timing,
    gnuWallMs: timing.wallMs,
    wallMs: measuredWallMs,
    stdoutBytes: execution.stdoutBytes,
    stderrBytes: execution.stderrBytes,
    inputBytes: options.inputBytes ?? 0,
    outputBytes: options.outputBytesFromStdout ? execution.stdoutBytes : 0,
  };
}

async function checked(config, program, args, options = {}) {
  const result = await execute(program, args, { ...options, timeoutMs: config.timeoutMs });
  if (result.code !== 0) throw new Error(`${program} ${args.join(' ')} failed: ${result.stderr.slice(-1000)}`);
}

async function checkedOutput(config, program, args) {
  const result = await execute(program, args, { timeoutMs: config.timeoutMs, captureStdout: true });
  if (result.code !== 0) throw new Error(`${program} ${args.join(' ')} failed: ${result.stderr.slice(-1000)}`);
  return result.stdout;
}

function execute(program, args, options = {}) {
  return new Promise((resolveCommand, rejectCommand) => {
    const child = spawn(program, args, { cwd: options.cwd, env: { ...process.env, GIT_TERMINAL_PROMPT: '0' }, stdio: ['pipe', 'pipe', 'pipe'] });
    let stdoutBytes = 0;
    let stderrBytes = 0;
    const stdout = [];
    const stderr = [];
    child.stdout.on('data', (chunk) => {
      stdoutBytes += chunk.length;
      if (options.captureStdout) stdout.push(chunk);
    });
    child.stderr.on('data', (chunk) => {
      stderrBytes += chunk.length;
      if (stderrBytes <= 1024 * 1024) stderr.push(chunk);
    });
    if (options.stdinPath) createReadStream(options.stdinPath).on('error', rejectCommand).pipe(child.stdin);
    else child.stdin.end();
    const timer = setTimeout(() => child.kill('SIGKILL'), options.timeoutMs || 60_000);
    child.on('error', rejectCommand);
    child.on('close', (code, signal) => {
      clearTimeout(timer);
      resolveCommand({
        code,
        signal,
        stdoutBytes,
        stderrBytes,
        stdout: Buffer.concat(stdout).toString('utf8'),
        stderr: Buffer.concat(stderr).toString('utf8'),
      });
    });
  });
}

async function prepareCache(config, caseRoot, sampleIndex) {
  if (config.evictBytes === 0) return sampleIndex === 0 ? 'first-touch' : 'warm';
  const path = join(caseRoot, 'cache-pressure.bin');
  const handle = await open(path, sampleIndex === 0 ? 'w+' : 'r');
  const chunk = Buffer.alloc(Math.min(config.evictBytes, 8 * 1024 * 1024), sampleIndex % 251);
  try {
    if (sampleIndex === 0) {
      let written = 0;
      while (written < config.evictBytes) {
        const size = Math.min(chunk.length, config.evictBytes - written);
        await handle.write(chunk, 0, size, written);
        written += size;
      }
    }
    let offset = 0;
    while (offset < config.evictBytes) {
      const size = Math.min(chunk.length, config.evictBytes - offset);
      await handle.read(chunk, 0, size, offset);
      offset += size;
    }
  } finally {
    await handle.close();
  }
  return 'evicted-by-pressure-file';
}

export function summarizeOperation(samples) {
  const median = (field) => {
    const values = samples.map((sample) => sample[field]).filter(Number.isFinite).sort((left, right) => left - right);
    return medianValue(values);
  };
  const normalizedMedian = (field) => {
    const values = samples.map((sample) => sample.normalized[field]).filter(Number.isFinite).sort((left, right) => left - right);
    return medianValue(values);
  };
  return {
    samples: samples.length,
    medianWallMs: median('wallMs'),
    medianCpuMs: median('cpuMs'),
    medianMaxRssKiB: median('maxRssKiB'),
    medianInputMiBPerSecond: normalizedMedian('inputMiBPerSecond'),
    medianCpuMsPerMiB: normalizedMedian('cpuMsPerMiB'),
    medianOutputToInputRatio: normalizedMedian('outputToInputRatio'),
  };
}

function medianValue(values) {
  if (!values.length) return null;
  const middle = Math.floor(values.length / 2);
  return round(values.length % 2 ? values[middle] : (values[middle - 1] + values[middle]) / 2);
}

export function parseCountObjects(value) {
  const fields = Object.fromEntries(value.trim().split('\n').map((line) => line.split(':').map((part) => part.trim())));
  return {
    count: Number(fields.count) || 0,
    sizeBytes: (Number(fields.size) || 0) * 1024,
    packs: Number(fields.packs) || 0,
    packedBytes: (Number(fields['size-pack']) || 0) * 1024,
  };
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

async function assertTools(config) {
  await checked(config, '/usr/bin/time', ['--version']);
  await checked(config, 'git', ['--version']);
}

async function hostFacts(outputRoot) {
  const disk = await statfs(outputRoot);
  const processors = cpus();
  return {
    platform: platform(),
    release: release(),
    cpuModel: processors[0]?.model || 'unknown',
    logicalCpuCount: processors.length,
    totalMemoryBytes: totalmem(),
    freeMemoryBytesAtStart: freemem(),
    filesystemBlockSize: disk.bsize,
    filesystemTotalBytes: disk.blocks * disk.bsize,
    filesystemFreeBytesAtStart: disk.bavail * disk.bsize,
    gitVersion: (await checkedOutput({ timeoutMs: 10_000 }, 'git', ['--version'])).trim(),
    outputFilesystemPath: resolve(outputRoot),
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
  const rows = report.cases.flatMap((entry) => entry.measurements.map(({ operation, summary }) =>
    `| ${entry.spec.label} | ${operation} | ${summary.medianWallMs} | ${summary.medianCpuMs} | ${summary.medianMaxRssKiB} | ${summary.medianInputMiBPerSecond ?? 'n/a'} | ${summary.medianCpuMsPerMiB ?? 'n/a'} | ${summary.medianOutputToInputRatio ?? 'n/a'} |`,
  )).join('\n');
  return `# Git component physics\n\nGenerated: ${report.generatedAt}\n\nHost: ${report.host.cpuModel}, ${report.host.logicalCpuCount} logical CPUs, ${bytesLabel(report.host.totalMemoryBytes)} RAM\n\n| Case | Operation | Wall ms | CPU ms | Max RSS KiB | Input MiB/s | CPU ms/MiB | Output/input |\n|---|---|---:|---:|---:|---:|---:|---:|\n${rows}\n\n## Limits\n\n${report.limitations.map((entry) => `- ${entry}`).join('\n')}\n`;
}

function parseOperations(value) {
  const operations = [...new Set(value.split(',').map((entry) => entry.trim()).filter(Boolean))];
  const unknown = operations.filter((entry) => !OPERATIONS.has(entry));
  if (unknown.length) throw new Error(`unsupported physics operations: ${unknown.join(', ')}`);
  if (operations.includes('index') && !operations.includes('pack')) throw new Error('index operation requires pack');
  return operations;
}

function positiveInteger(value, fallback) {
  const parsed = Number.parseInt(value || String(fallback), 10);
  if (!Number.isInteger(parsed) || parsed < 1) throw new Error('expected a positive integer');
  return parsed;
}

function message(error) { return error instanceof Error ? error.message : String(error); }
