#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';

const api = (process.env.SCOPE_BENCH_API_URL || 'http://localhost:8080').replace(/\/$/, '');
const token = process.env.SCOPE_BENCH_AUTH_TOKEN || '';
const count = number('SCOPE_BENCH_SAMPLES', 12);
const warmup = number('SCOPE_BENCH_WARMUP', 2);
const concurrency = number('SCOPE_BENCH_CONCURRENCY', 6);
const timeout = number('SCOPE_BENCH_TIMEOUT_MS', 15_000);
const outputRoot = resolve(process.env.SCOPE_BENCH_OUTPUT_DIR || '.tmp/bench/phase0');
const owner = handle(process.env.SCOPE_BENCH_OWNER || process.env.SCOPE_DEV_USER_HANDLE || 'dev');
const pushMatrix = process.env.SCOPE_BENCH_PUSH_MATRIX || 'smoke';
const caseFilter = process.env.SCOPE_BENCH_CASE || '';

await ready();
const results = [];
for (const entry of cases().filter((entry) => !caseFilter || entry.name.includes(caseFilter))) {
  process.stdout.write(`running ${entry.name}... `);
  const result = await measure(entry);
  results.push(result);
  console.log(result.ok ? 'ok' : 'failed');
}

const generatedAt = new Date().toISOString();
const report = {
  version: 2,
  phase: 'phase-0',
  generatedAt,
  apiUrl: api,
  owner,
  config: { count, warmup, concurrency, timeout, authenticated: Boolean(token), pushMatrix, caseFilter },
  cases: results,
  skipped: token ? [] : [{ name: 'authenticated reads and receive-pack', reason: 'auth token is not set' }],
};
const output = join(outputRoot, generatedAt.replaceAll(':', '-'));
await mkdir(output, { recursive: true });
await writeFile(join(output, 'results.json'), `${JSON.stringify(report, null, 2)}\n`);
await writeFile(join(output, 'summary.md'), markdown(report));
console.log(`results: ${join(output, 'results.json')}\nsummary: ${join(output, 'summary.md')}`);
if (results.some((result) => !result.ok)) process.exitCode = 1;

function cases() {
  const publicRepo = `/v1/repos/${encodeURIComponent(owner)}/public-demo`;
  const updateRepo = `/v1/repos/${encodeURIComponent(owner)}/update-demo`;
  const entries = [
    http('repo summary', publicRepo),
    http('repo files', `${publicRepo}/files`),
    http('projection preview', `${publicRepo}/projection-preview?audience=public&source=live`),
    http('commit history', `${publicRepo}/commits?audience=public`),
    { name: 'git ls-remote', command: ['git', 'ls-remote', `${api}/git/public/${owner}/public-demo`] },
  ];
  if (token) {
    entries.push(
      http('authenticated repo list', '/v1/repos', true),
      http('private projection preview', `${updateRepo}/projection-preview?audience=private&source=live`, true),
      pushCase('git push small first', { files: 100, bytesPerFile: 10 * 1024 }),
    );
    if (pushMatrix === 'full') entries.push(
      pushCase('git push many small first', { files: 10_000, bytesPerFile: 5 * 1024 }),
      pushCase('git push one-file large-tree delta', { files: 10_000, bytesPerFile: 256, changedFiles: 1 }),
      pushCase('git push deep-history delta', { files: 10, bytesPerFile: 256, history: 1_000, changedFiles: 1 }),
      pushCase('git push wide delta', { files: 2_000, bytesPerFile: 256, changedFiles: 1_000 }),
    );
  }
  return entries;
}

function pushCase(name, fixture) {
  return { name, push: fixture };
}

function http(name, path, auth = false) {
  return { name, url: `${api}${path}`, auth };
}

async function measure(entry) {
  const sample = () => entry.push ? push(entry.push) : entry.url ? request(entry) : command(entry.command);
  await many(warmup, sample);
  const sequential = await many(count, sample);
  const burst = entry.url
    ? (await many(count, () => Promise.all(Array.from({ length: concurrency }, sample)))).flat()
    : [];
  const values = [...sequential, ...burst];
  return {
    name: entry.name,
    ok: values.every((value) => value.ok),
    sequential: stats(sequential),
    burst: entry.url ? stats(burst) : null,
    failures: values.filter((value) => !value.ok).slice(0, 5),
  };
}

async function many(size, operation) {
  const values = [];
  for (let index = 0; index < size; index += 1) values.push(await operation());
  return values;
}

async function request(entry) {
  const started = performance.now();
  try {
    const headers = { accept: 'application/json' };
    if (entry.auth) headers.authorization = `Bearer ${token}`;
    const response = await fetch(entry.url, { headers, signal: AbortSignal.timeout(timeout) });
    const bytes = (await response.arrayBuffer()).byteLength;
    return sample(response.ok, started, response.status, bytes, response.ok ? null : `HTTP ${response.status}`);
  } catch (error) {
    return sample(false, started, null, 0, message(error));
  }
}

async function push(spec) {
  let fixture;
  try {
    fixture = await pushFixture(spec);
    const result = await command(
      ['git', '-c', 'push.recurseSubmodules=no', 'push', fixture.remote, `HEAD:${fixture.branch}`],
      fixture.dir,
      authEnvironment(fixture.remote, fixture.pushToken, fixture.pushIntent),
    );
    return result;
  } catch (error) {
    return sample(false, performance.now(), null, 0, message(error));
  } finally {
    if (fixture) await Promise.allSettled([
      apiJson(`/v1/repos/${encodeURIComponent(fixture.owner)}/${encodeURIComponent(fixture.repo)}`, { method: 'DELETE' }),
      rm(fixture.dir, { recursive: true, force: true }),
    ]);
  }
}

async function pushFixture(spec) {
  await mkdir(outputRoot, { recursive: true });
  const created = await apiJson('/v1/repos', {
    method: 'POST',
    body: { name: `bench-${Date.now()}-${Math.random().toString(16).slice(2)}`, visibility: 'Public' },
  });
  const fixture = {
    owner: created.repo.owner_handle,
    repo: created.repo.name,
    remote: new URL(new URL(created.init.git_remote_url).pathname, `${api}/`).toString(),
    branch: created.init.push_branch || 'main',
    pushToken: created.init.token?.secret ?? created.init.push_token?.secret,
    dir: await mkdtemp(join(outputRoot, 'push-')),
  };
  for (const args of [
    ['init'],
    ['symbolic-ref', 'HEAD', 'refs/heads/main'],
    ['config', 'user.email', 'bench@scope.local'],
    ['config', 'user.name', 'Scope Bench'],
  ]) await checkedGit(args, fixture.dir);
  const payload = 'x'.repeat(Math.max(1, spec.bytesPerFile - 1)) + '\n';
  for (let offset = 0; offset < spec.files; offset += 250) {
    const writes = [];
    for (let index = offset; index < Math.min(spec.files, offset + 250); index += 1) {
      const dir = join(fixture.dir, 'fixture', String(Math.floor(index / 100)));
      await mkdir(dir, { recursive: true });
      writes.push(writeFile(join(dir, `${String(index).padStart(6, '0')}.txt`), payload));
    }
    await Promise.all(writes);
  }
  await checkedGit(['add', '--all'], fixture.dir);
  await checkedGit(['commit', '-m', 'Benchmark fixture'], fixture.dir);
  for (let index = 0; index < (spec.history || 0); index += 1) {
    await checkedGit(['commit', '--allow-empty', '-m', `History ${index + 1}`], fixture.dir);
  }
  const config = await apiJson(`/v1/repos/${fixture.owner}/${fixture.repo}/config`);
  const headOid = await gitOutput(['rev-parse', 'HEAD'], fixture.dir);
  const intent = await apiJson(`/v1/repos/${fixture.owner}/${fixture.repo}/push-intents`, {
    method: 'POST',
    body: { head_oid: headOid, base_config_hash: config.config_hash, config: config.config },
  });
  fixture.pushIntent = intent.token;
  if (spec.changedFiles) {
    const initial = await command(
      ['git', '-c', 'push.recurseSubmodules=no', 'push', fixture.remote, `HEAD:${fixture.branch}`],
      fixture.dir,
      authEnvironment(fixture.remote, fixture.pushToken, fixture.pushIntent),
    );
    if (!initial.ok) throw new Error(initial.error || 'initial benchmark push failed');
    fixture.pushToken = token;
    for (let index = 0; index < spec.changedFiles; index += 1) {
      const path = join(fixture.dir, 'fixture', String(Math.floor(index / 100)), `${String(index).padStart(6, '0')}.txt`);
      await writeFile(path, `${payload.trimEnd()} updated\n`);
    }
    await checkedGit(['add', '--all'], fixture.dir);
    await checkedGit(['commit', '-m', `Update ${spec.changedFiles} files`], fixture.dir);
    const nextConfig = await apiJson(`/v1/repos/${fixture.owner}/${fixture.repo}/config`);
    const nextHead = await gitOutput(['rev-parse', 'HEAD'], fixture.dir);
    const nextIntent = await apiJson(`/v1/repos/${fixture.owner}/${fixture.repo}/push-intents`, {
      method: 'POST',
      body: { head_oid: nextHead, base_config_hash: nextConfig.config_hash, config: nextConfig.config },
    });
    fixture.pushIntent = nextIntent.token;
  }
  return fixture;
}

async function checkedGit(args, cwd) {
  const result = await command(['git', ...args], cwd);
  if (!result.ok) throw new Error(result.error || `git ${args.join(' ')} failed`);
}

async function gitOutput(args, cwd) {
  const result = await new Promise((resolve, reject) => {
    const child = spawn('git', args, { cwd, stdio: ['ignore', 'pipe', 'pipe'] });
    const chunks = [];
    child.stdout.on('data', (chunk) => chunks.push(chunk));
    child.on('error', reject);
    child.on('close', (code) => code === 0 ? resolve(Buffer.concat(chunks).toString('utf8').trim()) : reject(new Error(`git ${args.join(' ')} failed`)));
  });
  return result;
}

function command([program, ...args], cwd, extraEnv) {
  const started = performance.now();
  return new Promise((resolveSample) => {
    const child = spawn(program, args, { cwd, env: { ...process.env, ...extraEnv }, stdio: ['ignore', 'pipe', 'pipe'] });
    const chunks = [];
    child.stdout.on('data', (chunk) => chunks.push(chunk));
    child.stderr.on('data', (chunk) => chunks.push(chunk));
    const timer = setTimeout(() => child.kill('SIGKILL'), timeout);
    child.on('error', (error) => resolveSample(sample(false, started, null, 0, error.message)));
    child.on('close', (code, signal) => {
      clearTimeout(timer);
      const bytes = chunks.reduce((total, chunk) => total + chunk.length, 0);
      const error = code === 0 ? null : Buffer.concat(chunks).toString('utf8').slice(-1000) || String(signal);
      resolveSample(sample(code === 0, started, code, bytes, error));
    });
  });
}

async function apiJson(path, options = {}) {
  const headers = { accept: 'application/json', authorization: `Bearer ${token}` };
  if (options.body) headers['content-type'] = 'application/json';
  const response = await fetch(`${api}${path}`, {
    method: options.method || 'GET', headers,
    body: options.body ? JSON.stringify(options.body) : undefined,
    signal: AbortSignal.timeout(timeout),
  });
  const body = await response.text();
  if (!response.ok) throw new Error(`${options.method || 'GET'} ${path}: HTTP ${response.status} ${body.slice(0, 300)}`);
  return body ? JSON.parse(body) : null;
}

function authEnvironment(destination, secret, pushIntent) {
  const index = Number.parseInt(process.env.GIT_CONFIG_COUNT || '0', 10) || 0;
  return {
    GIT_CONFIG_COUNT: String(index + 2),
    [`GIT_CONFIG_KEY_${index}`]: `http.${destination}.extraHeader`,
    [`GIT_CONFIG_VALUE_${index}`]: `Authorization: Bearer ${secret}`,
    [`GIT_CONFIG_KEY_${index + 1}`]: `http.${destination}.extraHeader`,
    [`GIT_CONFIG_VALUE_${index + 1}`]: `X-Scope-Push-Intent: ${pushIntent}`,
  };
}

function sample(ok, started, status, bytes, error) {
  return { ok, durationMs: performance.now() - started, status, bytes, error };
}

function stats(values) {
  const durations = values.map(({ durationMs }) => durationMs).sort((a, b) => a - b);
  const at = (point) => round(durations[Math.max(0, Math.ceil(durations.length * point) - 1)] || 0);
  return {
    count: values.length,
    ok: values.filter(({ ok }) => ok).length,
    meanMs: round(durations.reduce((sum, value) => sum + value, 0) / durations.length),
    p50Ms: at(0.5), p95Ms: at(0.95),
    bytes: values.reduce((sum, value) => sum + value.bytes, 0),
  };
}

function markdown(value) {
  const rows = value.cases.map((entry) => `| ${entry.name} | ${entry.ok ? 'yes' : 'no'} | ${entry.sequential.meanMs} | ${entry.sequential.p95Ms} |`).join('\n');
  return `# Scope benchmark\n\nGenerated: ${value.generatedAt}\n\n| Case | OK | Mean ms | p95 ms |\n|---|---:|---:|---:|\n${rows}\n`;
}

async function ready() {
  const response = await fetch(`${api}/readyz`, { signal: AbortSignal.timeout(timeout) });
  if (!response.ok) throw new Error(`API is not ready at ${api}`);
}

function number(name, fallback) {
  const value = Number.parseInt(process.env[name] || String(fallback), 10);
  if (!Number.isInteger(value) || value < 1) throw new Error(`${name} must be positive`);
  return value;
}

function handle(value) { return value.toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-|-$/g, '') || 'dev'; }
function round(value) { return Math.round(value * 100) / 100; }
function message(error) { return error instanceof Error ? error.message : String(error); }
