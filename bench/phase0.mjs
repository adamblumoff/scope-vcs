#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(SCRIPT_DIR, '..');
const ROOT_ENV = readDotenv(join(ROOT, '.env.local'));
const apiUrl = trimTrailingSlash(
  envValue('SCOPE_BENCH_API_URL') || 'http://localhost:8080',
);
const explicitAuthToken = envValue('SCOPE_BENCH_AUTH_TOKEN');
const cliSessionToken = explicitAuthToken ? '' : readStoredCliSessionToken(apiUrl);
let fixtureCounter = 0;

const config = {
  apiUrl,
  warmup: intEnv('SCOPE_BENCH_WARMUP', 2),
  samples: intEnv('SCOPE_BENCH_SAMPLES', 12),
  concurrency: intEnv('SCOPE_BENCH_CONCURRENCY', 6),
  timeoutMs: intEnv('SCOPE_BENCH_TIMEOUT_MS', 15_000),
  pushFiles: intEnv('SCOPE_BENCH_PUSH_FILES', 4),
  pushCommits: intEnv('SCOPE_BENCH_PUSH_COMMITS', 2),
  pushBurst: boolEnv('SCOPE_BENCH_PUSH_BURST', false),
  authToken: explicitAuthToken || cliSessionToken,
  authTokenSource: explicitAuthToken
    ? 'SCOPE_BENCH_AUTH_TOKEN'
    : cliSessionToken
      ? 'local Scope CLI session'
      : 'none',
  outputRoot: envValue('SCOPE_BENCH_OUTPUT_DIR')
    ? resolve(envValue('SCOPE_BENCH_OUTPUT_DIR'))
    : join(ROOT, '.tmp', 'bench', 'phase0'),
};

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});

async function main() {
  validateConfig(config);

  await requireReady(config.apiUrl);
  const owner = await discoverOwner(config.apiUrl, ownerCandidates());
  const cases = benchmarkCases(owner);
  const skipped = skippedCases();
  const startedAt = new Date();
  const results = [];

  for (const benchCase of cases) {
    process.stdout.write(`running ${benchCase.name}... `);
    const result = await runCase(benchCase);
    results.push(result);
    process.stdout.write(`${result.ok ? 'ok' : 'failed'}\n`);
  }

  const report = {
    version: 1,
    phase: 'phase-0',
    generatedAt: startedAt.toISOString(),
    apiUrl: config.apiUrl,
    owner,
    config: {
      warmup: config.warmup,
      samples: config.samples,
      concurrency: config.concurrency,
      timeoutMs: config.timeoutMs,
      pushFiles: config.pushFiles,
      pushCommits: config.pushCommits,
      pushBurst: config.pushBurst,
      authenticatedCases: Boolean(config.authToken),
      authTokenSource: config.authTokenSource,
    },
    environment: {
      node: process.version,
      platform: process.platform,
      arch: process.arch,
    },
    cases: results,
    skipped,
    instrumentationGaps: [
      'Object-store byte counters are not exposed by the local API.',
      'Projection cache hit rates are not exposed by the local API.',
      'Internal API Git subprocess counts are not exposed by the local API.',
    ],
  };

  const outputDir = join(config.outputRoot, timestampSlug(startedAt));
  await mkdir(outputDir, { recursive: true });
  const jsonPath = join(outputDir, 'results.json');
  const markdownPath = join(outputDir, 'summary.md');
  await writeFile(jsonPath, `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(markdownPath, markdownSummary(report));

  printSummary(report, jsonPath, markdownPath);

  if (results.some((result) => !result.ok)) {
    process.exitCode = 1;
  }
}

function validateConfig(input) {
  for (const [name, value] of [
    ['SCOPE_BENCH_WARMUP', input.warmup],
    ['SCOPE_BENCH_SAMPLES', input.samples],
    ['SCOPE_BENCH_CONCURRENCY', input.concurrency],
    ['SCOPE_BENCH_TIMEOUT_MS', input.timeoutMs],
  ]) {
    if (!Number.isInteger(value) || value < 1) {
      throw new Error(`${name} must be a positive integer`);
    }
  }

  if (!Number.isInteger(input.pushFiles) || input.pushFiles < 1) {
    throw new Error('SCOPE_BENCH_PUSH_FILES must be a positive integer');
  }
  if (!Number.isInteger(input.pushCommits) || input.pushCommits < 1) {
    throw new Error('SCOPE_BENCH_PUSH_COMMITS must be a positive integer');
  }
}

async function requireReady(apiUrl) {
  const result = await sampleFetch({
    name: 'readyz',
    method: 'GET',
    url: `${apiUrl}/readyz`,
    expectedStatuses: [200],
  });
  if (!result.ok) {
    throw new Error(
      `local API is not ready at ${apiUrl}/readyz; run ./dev/scope-dev up first`,
    );
  }
}

async function discoverOwner(apiUrl, candidates) {
  for (const owner of candidates) {
    const result = await sampleFetch({
      name: 'owner discovery',
      method: 'GET',
      url: `${apiUrl}/v1/repos/${encodeURIComponent(owner)}/public-demo`,
      expectedStatuses: [200],
    });
    if (result.ok) {
      return owner;
    }
  }

  throw new Error(
    `could not find seeded public-demo owner; tried ${candidates.join(', ')}`,
  );
}

function ownerCandidates() {
  const explicitOwner = envValue('SCOPE_BENCH_OWNER');
  if (explicitOwner) {
    return [explicitOwner];
  }

  const candidates = [];
  addCandidate(candidates, envValue('SCOPE_DEV_USER_HANDLE'));
  const email = envValue('SCOPE_DEV_USER_EMAIL');
  if (email.includes('@')) {
    addCandidate(candidates, normalizeHandle(email.split('@')[0]));
  }
  addCandidate(candidates, 'dev-user');
  addCandidate(candidates, 'dev');
  return candidates;
}

function addCandidate(candidates, value) {
  if (value && !candidates.includes(value)) {
    candidates.push(value);
  }
}

function benchmarkCases(owner) {
  const publicRepo = 'public-demo';
  const updateRepo = 'update-demo';
  const publicBase = `/v1/repos/${segment(owner)}/${segment(publicRepo)}`;
  const updateBase = `/v1/repos/${segment(owner)}/${segment(updateRepo)}`;

  const cases = [
    httpCase('repo summary public-demo', publicBase),
    httpCase('repo files public-demo', `${publicBase}/files`),
    httpCase('projection public-demo', `${publicBase}/projections`),
    httpCase(
      'projection preview public-demo',
      `${publicBase}/projection-preview?audience=public&source=live`,
    ),
    httpCase('commit history public-demo', `${publicBase}/commits?audience=public`),
    httpCase(
      'git info refs public-demo',
      `/git/${segment(owner)}/${segment(publicRepo)}/info/refs?service=git-upload-pack`,
    ),
    httpCase('repo summary update-demo', updateBase),
    httpCase(
      'projection preview update-demo',
      `${updateBase}/projection-preview?audience=public&source=live`,
    ),
    {
      name: 'git ls-remote public-demo',
      kind: 'command',
      command: 'git',
      args: ['ls-remote', `${config.apiUrl}/git/${owner}/${publicRepo}`],
      burst: false,
    },
  ];

  if (config.authToken) {
    cases.push(
      httpCase('auth list repos', '/v1/repos', { auth: true }),
      httpCase('auth settings public-demo', `${publicBase}/settings`, { auth: true }),
      httpCase('auth staged update update-demo', `${updateBase}/staged-update`, {
        auth: true,
      }),
      httpCase(
        'auth owner projection preview update-demo',
        `${updateBase}/projection-preview?audience=owner&source=live`,
        { auth: true },
      ),
      {
        name: 'git receive-pack first push',
        kind: 'push-fixture',
        burst: config.pushBurst,
      },
    );
  }

  return cases;
}

function skippedCases() {
  if (config.authToken) {
    return [];
  }

  return [
    {
      name: 'owner/review API routes',
      reason: unauthenticatedSkipReason('include authenticated read-only cases'),
    },
    {
      name: 'git receive-pack push benchmark',
      reason: unauthenticatedSkipReason('create disposable push fixtures'),
    },
  ];
}

function unauthenticatedSkipReason(action) {
  if (process.platform === 'darwin' || process.platform === 'win32') {
    return `set SCOPE_BENCH_AUTH_TOKEN to ${action}; local CLI session auto-detection is only supported for Linux file-backed sessions`;
  }
  return `run scope login against the local API or set SCOPE_BENCH_AUTH_TOKEN to ${action}`;
}

function httpCase(name, path, options = {}) {
  return {
    name,
    kind: 'http',
    method: 'GET',
    url: `${config.apiUrl}${path}`,
    expectedStatuses: options.expectedStatuses || [200],
    auth: Boolean(options.auth),
    burst: options.burst !== false,
  };
}

async function runCase(benchCase) {
  const warmup = [];
  for (let index = 0; index < config.warmup; index += 1) {
    warmup.push(await runSample(benchCase));
  }

  const sequential = [];
  for (let index = 0; index < config.samples; index += 1) {
    sequential.push(await runSample(benchCase));
  }

  let burst = [];
  if (benchCase.burst) {
    for (let round = 0; round < config.samples; round += 1) {
      const samples = await Promise.all(
        Array.from({ length: config.concurrency }, () => runSample(benchCase)),
      );
      burst = burst.concat(samples);
    }
  }

  return {
    name: benchCase.name,
    kind: benchCase.kind,
    target: targetForCase(benchCase),
    ok:
      warmup.every((sample) => sample.ok) &&
      sequential.every((sample) => sample.ok) &&
      burst.every((sample) => sample.ok),
    warmup: summarizeSamples(warmup),
    sequential: summarizeSamples(sequential),
    burst: benchCase.burst ? summarizeSamples(burst) : null,
    failures: warmup
      .concat(sequential, burst)
      .filter((sample) => !sample.ok)
      .slice(0, 5),
  };
}

function runSample(benchCase) {
  if (benchCase.kind === 'http') {
    return sampleFetch(benchCase);
  }
  if (benchCase.kind === 'push-fixture') {
    return samplePushFixture(benchCase);
  }
  return sampleCommand(benchCase);
}

async function sampleFetch(benchCase) {
  const started = performance.now();
  try {
    const headers = { accept: 'application/json' };
    if (benchCase.auth) {
      headers.authorization = `Bearer ${config.authToken}`;
    }
    const response = await fetch(benchCase.url, {
      method: benchCase.method,
      headers,
      signal: timeoutSignal(config.timeoutMs),
    });
    const body = Buffer.from(await response.arrayBuffer());
    const durationMs = performance.now() - started;
    const ok = benchCase.expectedStatuses.includes(response.status);

    return {
      ok,
      durationMs,
      status: response.status,
      bytes: body.length,
      error: ok ? null : `expected ${benchCase.expectedStatuses.join('/')} got ${response.status}`,
    };
  } catch (error) {
    return {
      ok: false,
      durationMs: performance.now() - started,
      status: null,
      bytes: 0,
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

function sampleCommand(benchCase) {
  const started = performance.now();
  return new Promise((resolveSample) => {
    const child = spawn(benchCase.command, benchCase.args, {
      cwd: benchCase.cwd,
      env: benchCase.env ? { ...process.env, ...benchCase.env } : process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdoutBytes = 0;
    let stderr = '';
    let finished = false;
    const timeout = setTimeout(() => {
      if (!finished) {
        child.kill('SIGKILL');
      }
    }, config.timeoutMs);
    timeout.unref?.();

    child.stdout.on('data', (chunk) => {
      stdoutBytes += chunk.length;
    });
    child.stderr.on('data', (chunk) => {
      stderr = `${stderr}${chunk.toString('utf8')}`.slice(-2000);
    });
    child.on('error', (error) => {
      finished = true;
      clearTimeout(timeout);
      resolveSample({
        ok: false,
        durationMs: performance.now() - started,
        status: null,
        bytes: stdoutBytes + Buffer.byteLength(stderr),
        error: error.message,
      });
    });
    child.on('close', (code, signal) => {
      finished = true;
      clearTimeout(timeout);
      const ok = code === 0;
      resolveSample({
        ok,
        durationMs: performance.now() - started,
        status: code,
        signal,
        bytes: stdoutBytes + Buffer.byteLength(stderr),
        error: ok ? null : stderr.trim() || `process exited with ${code ?? signal}`,
      });
    });
  });
}

async function samplePushFixture() {
  let fixture = null;
  try {
    fixture = await createPushFixture();
    const pushSample = await sampleCommand({
      name: 'git receive-pack first push',
      kind: 'command',
      command: 'git',
      args: [
        '-c',
        'push.recurseSubmodules=no',
        'push',
        fixture.remoteUrl,
        `HEAD:${fixture.branch}`,
      ],
      cwd: fixture.dir,
      env: gitPushAuthEnv(fixture.remoteUrl, fixture.pushToken),
    });
    const cleanupErrors = await cleanupPushFixture(fixture);
    if (cleanupErrors.length > 0) {
      const cleanupMessage = `push fixture cleanup failed: ${cleanupErrors.join('; ')}`;
      return {
        ...pushSample,
        ok: false,
        error: pushSample.error
          ? `${pushSample.error}; ${cleanupMessage}`
          : cleanupMessage,
      };
    }
    return pushSample;
  } catch (error) {
    const cleanupErrors = fixture ? await cleanupPushFixture(fixture) : [];
    const message = error instanceof Error ? error.message : String(error);
    return {
      ok: false,
      durationMs: 0,
      status: null,
      bytes: 0,
      error: cleanupErrors.length
        ? `${message}; cleanup failed: ${cleanupErrors.join('; ')}`
        : message,
    };
  }
}

async function createPushFixture() {
  if (!config.authToken) {
    throw new Error('auth token required for git receive-pack benchmark');
  }

  await mkdir(config.outputRoot, { recursive: true });
  const repoName = nextFixtureRepoName();
  const fixture = {
    owner: '',
    repo: '',
    remoteUrl: '',
    branch: 'main',
    pushToken: '',
    dir: '',
  };

  try {
    const created = await apiJson('/v1/repos', {
      method: 'POST',
      auth: true,
      body: {
        name: repoName,
        visibility: 'Public',
      },
    });
    fixture.owner = created?.repo?.owner_handle || '';
    fixture.repo = created?.repo?.name || '';
    fixture.remoteUrl = created?.init?.git_remote_url || '';
    fixture.branch = created?.init?.push_branch || 'main';
    fixture.pushToken = created?.init?.push_token?.secret || '';
    fixture.dir = await mkdtemp(join(config.outputRoot, 'push-work-'));
    if (
      !fixture.owner ||
      !fixture.repo ||
      !fixture.remoteUrl ||
      !fixture.pushToken
    ) {
      throw new Error('create repo response did not include push fixture details');
    }

    await initializePushFixtureRepo(fixture.dir, fixture.repo);
    return fixture;
  } catch (error) {
    await cleanupPushFixture(fixture);
    throw error;
  }
}

function nextFixtureRepoName() {
  fixtureCounter += 1;
  return `phase0-push-${Date.now()}-${process.pid}-${fixtureCounter}`;
}

async function initializePushFixtureRepo(dir, repoName) {
  await checkedCommand('git', ['init'], dir);
  await checkedCommand('git', ['symbolic-ref', 'HEAD', 'refs/heads/main'], dir);
  await checkedCommand('git', ['config', 'user.email', 'bench@scope.local'], dir);
  await checkedCommand('git', ['config', 'user.name', 'Scope Bench'], dir);

  for (let commitIndex = 0; commitIndex < config.pushCommits; commitIndex += 1) {
    await writePushFixtureFiles(dir, repoName, commitIndex);
    await checkedCommand('git', ['add', '--all'], dir);
    await checkedCommand(
      'git',
      ['commit', '-m', `Phase 0 push fixture ${commitIndex + 1}`],
      dir,
    );
  }
}

async function writePushFixtureFiles(dir, repoName, commitIndex) {
  await mkdir(join(dir, 'src'), { recursive: true });
  await writeFile(
    join(dir, 'README.md'),
    [
      `# ${repoName}`,
      '',
      'Disposable Phase 0 push benchmark fixture.',
      `Commit: ${commitIndex + 1}`,
      '',
    ].join('\n'),
  );

  for (let fileIndex = 0; fileIndex < config.pushFiles; fileIndex += 1) {
    await writeFile(
      join(dir, 'src', `file-${fileIndex + 1}.txt`),
      [
        `repo=${repoName}`,
        `commit=${commitIndex + 1}`,
        `file=${fileIndex + 1}`,
        `payload=${'x'.repeat(128)}`,
        '',
      ].join('\n'),
    );
  }
}

async function checkedCommand(command, args, cwd) {
  const sample = await sampleCommand({
    name: `${command} ${args.join(' ')}`,
    kind: 'command',
    command,
    args,
    cwd,
  });
  if (!sample.ok) {
    throw new Error(sample.error || `${command} ${args.join(' ')} failed`);
  }
}

async function cleanupPushFixture(fixture) {
  const errors = [];
  if (fixture.owner && fixture.repo) {
    try {
      await apiJson(`/v1/repos/${segment(fixture.owner)}/${segment(fixture.repo)}`, {
        method: 'DELETE',
        auth: true,
      });
    } catch (error) {
      errors.push(error instanceof Error ? error.message : String(error));
    }
  }
  if (fixture.dir) {
    try {
      await rm(fixture.dir, { recursive: true, force: true });
    } catch (error) {
      errors.push(error instanceof Error ? error.message : String(error));
    }
  }
  return errors;
}

async function apiJson(path, options = {}) {
  const headers = { accept: 'application/json' };
  if (options.auth) {
    headers.authorization = `Bearer ${config.authToken}`;
  }
  if (options.body !== undefined) {
    headers['content-type'] = 'application/json';
  }

  const response = await fetch(`${config.apiUrl}${path}`, {
    method: options.method || 'GET',
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
    signal: timeoutSignal(config.timeoutMs),
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(
      `${options.method || 'GET'} ${path} returned HTTP ${response.status}: ${text.slice(0, 300)}`,
    );
  }
  return text ? JSON.parse(text) : null;
}

function gitPushAuthEnv(destination, bearerToken) {
  const inheritedConfigCount = Number.parseInt(
    process.env.GIT_CONFIG_COUNT || '0',
    10,
  );
  const configIndex = Number.isInteger(inheritedConfigCount)
    ? inheritedConfigCount
    : 0;
  return {
    GIT_CONFIG_COUNT: String(configIndex + 1),
    [`GIT_CONFIG_KEY_${configIndex}`]: `http.${destination}.extraHeader`,
    [`GIT_CONFIG_VALUE_${configIndex}`]: `Authorization: Bearer ${bearerToken}`,
  };
}

function summarizeSamples(samples) {
  const okSamples = samples.filter((sample) => sample.ok);
  const durations = okSamples
    .map((sample) => sample.durationMs)
    .sort((left, right) => left - right);
  const bytes = okSamples.map((sample) => sample.bytes);

  return {
    requests: samples.length,
    ok: okSamples.length,
    failed: samples.length - okSamples.length,
    minMs: percentile(durations, 0),
    p50Ms: percentile(durations, 0.5),
    p95Ms: percentile(durations, 0.95),
    maxMs: percentile(durations, 1),
    meanMs: mean(durations),
    meanBytes: mean(bytes),
    statuses: statusCounts(okSamples),
  };
}

function percentile(sortedValues, point) {
  if (sortedValues.length === 0) {
    return null;
  }
  if (point === 0) {
    return round(sortedValues[0]);
  }
  const index = Math.min(
    sortedValues.length - 1,
    Math.max(0, Math.ceil(sortedValues.length * point) - 1),
  );
  return round(sortedValues[index]);
}

function mean(values) {
  if (values.length === 0) {
    return null;
  }
  return round(values.reduce((sum, value) => sum + value, 0) / values.length);
}

function statusCounts(samples) {
  const counts = {};
  for (const sample of samples) {
    const key = sample.status === null ? 'error' : String(sample.status);
    counts[key] = (counts[key] || 0) + 1;
  }
  return counts;
}

function markdownSummary(report) {
  const lines = [
    '# Phase 0 Benchmark Summary',
    '',
    `Generated: ${report.generatedAt}`,
    `API: ${report.apiUrl}`,
    `Owner: ${report.owner}`,
    `Config: warmup ${report.config.warmup}, samples ${report.config.samples}, concurrency ${report.config.concurrency}, timeout ${report.config.timeoutMs}ms`,
    `Push fixture: ${report.config.pushCommits} commits, ${report.config.pushFiles} files, burst ${report.config.pushBurst ? 'on' : 'off'}`,
    `Auth token source: ${report.config.authTokenSource}`,
    '',
    '| Case | Mode | OK | p50 ms | p95 ms | Mean ms | Mean bytes | Statuses |',
    '| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |',
  ];

  for (const benchCase of report.cases) {
    lines.push(summaryRow(benchCase.name, 'sequential', benchCase.sequential));
    if (benchCase.burst) {
      lines.push(summaryRow(benchCase.name, 'burst', benchCase.burst));
    }
  }

  lines.push('', '## Skipped');
  if (report.skipped.length === 0) {
    lines.push('- None');
  } else {
    for (const skipped of report.skipped) {
      lines.push(`- ${skipped.name}: ${skipped.reason}`);
    }
  }

  lines.push('', '## Instrumentation Gaps');
  for (const gap of report.instrumentationGaps) {
    lines.push(`- ${gap}`);
  }

  return `${lines.join('\n')}\n`;
}

function summaryRow(name, mode, summary) {
  return [
    escapePipe(name),
    mode,
    `${summary.ok}/${summary.requests}`,
    value(summary.p50Ms),
    value(summary.p95Ms),
    value(summary.meanMs),
    value(summary.meanBytes),
    escapePipe(JSON.stringify(summary.statuses)),
  ].join(' | ').replace(/^/, '| ').replace(/$/, ' |');
}

function printSummary(report, jsonPath, markdownPath) {
  console.log('');
  console.log(`Phase 0 benchmark complete for ${report.owner} at ${report.apiUrl}`);
  console.log('');
  console.log('Case | Mode | OK | p50 ms | p95 ms | Mean ms | Mean bytes | Statuses');
  console.log('--- | --- | ---: | ---: | ---: | ---: | ---: | ---');
  for (const benchCase of report.cases) {
    console.log(summaryRow(benchCase.name, 'sequential', benchCase.sequential));
    if (benchCase.burst) {
      console.log(summaryRow(benchCase.name, 'burst', benchCase.burst));
    }
  }

  if (report.skipped.length > 0) {
    console.log('');
    console.log('Skipped:');
    for (const skipped of report.skipped) {
      console.log(`- ${skipped.name}: ${skipped.reason}`);
    }
  }

  console.log('');
  console.log(`JSON: ${jsonPath}`);
  console.log(`Markdown: ${markdownPath}`);
}

function readDotenv(path) {
  const values = new Map();
  if (!existsSync(path)) {
    return values;
  }

  const lines = readFileSync(path, 'utf8').split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) {
      continue;
    }
    const match = trimmed.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/);
    if (!match) {
      continue;
    }
    values.set(match[1], unquote(match[2].trim()));
  }
  return values;
}

function unquote(value) {
  if (
    (value.startsWith('"') && value.endsWith('"')) ||
    (value.startsWith("'") && value.endsWith("'"))
  ) {
    return value.slice(1, -1);
  }
  return value;
}

function envValue(name) {
  return process.env[name] || ROOT_ENV.get(name) || '';
}

function readStoredCliSessionToken(apiUrl) {
  if (process.platform === 'darwin' || process.platform === 'win32') {
    // The CLI stores sessions in the OS keychain on macOS/Windows. Keep this
    // harness dependency-free and use SCOPE_BENCH_AUTH_TOKEN on those systems.
    return '';
  }

  const configDir = localConfigDir();
  if (!configDir) {
    return '';
  }

  const sessionPath = join(
    configDir,
    'scope',
    'sessions',
    sessionStorageKey(apiUrl),
  );
  try {
    const token = readFileSync(sessionPath, 'utf8').trim();
    return token.startsWith('scope_cli_') ? token : '';
  } catch {
    return '';
  }
}

function localConfigDir() {
  if (process.env.XDG_CONFIG_HOME) {
    return process.env.XDG_CONFIG_HOME;
  }
  if (process.env.HOME) {
    return join(process.env.HOME, '.config');
  }
  if (process.env.USERPROFILE) {
    return join(process.env.USERPROFILE, '.config');
  }
  return '';
}

function sessionStorageKey(apiUrl) {
  return `cli-session-${Buffer.from(apiUrl, 'utf8').toString('hex')}`;
}

function normalizeHandle(value) {
  let handle = '';
  let lastWasSeparator = false;
  for (const char of value.trim()) {
    const code = char.charCodeAt(0);
    if (
      (code >= 48 && code <= 57) ||
      (code >= 65 && code <= 90) ||
      (code >= 97 && code <= 122)
    ) {
      handle += char.toLowerCase();
      lastWasSeparator = false;
    } else if ((char === '-' || char === '_') && !lastWasSeparator) {
      handle += '-';
      lastWasSeparator = true;
    }
  }

  handle = handle.replace(/^-+|-+$/g, '');
  return handle && handle.length <= 40 ? handle : null;
}

function intEnv(name, fallback) {
  const raw = envValue(name);
  if (!raw) {
    return fallback;
  }
  return Number.parseInt(raw, 10);
}

function boolEnv(name, fallback) {
  const raw = envValue(name).trim().toLowerCase();
  if (!raw) {
    return fallback;
  }
  return ['1', 'true', 'yes', 'on'].includes(raw);
}

function timeoutSignal(ms) {
  if (typeof AbortSignal.timeout === 'function') {
    return AbortSignal.timeout(ms);
  }
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), ms);
  timeout.unref?.();
  return controller.signal;
}

function timestampSlug(date) {
  return date.toISOString().replace(/[:.]/g, '-');
}

function trimTrailingSlash(value) {
  return value.replace(/\/+$/, '');
}

function segment(value) {
  return encodeURIComponent(value);
}

function commandString(benchCase) {
  return [benchCase.command, ...benchCase.args].join(' ');
}

function targetForCase(benchCase) {
  if (benchCase.kind === 'http') {
    return benchCase.url;
  }
  if (benchCase.kind === 'push-fixture') {
    return 'disposable repo first push via git receive-pack';
  }
  return commandString(benchCase);
}

function value(input) {
  return input === null ? 'n/a' : String(input);
}

function escapePipe(input) {
  return String(input).replace(/\|/g, '\\|');
}

function round(value) {
  return Math.round(value * 100) / 100;
}
