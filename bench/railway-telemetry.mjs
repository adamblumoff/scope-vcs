#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import { percentile, round } from './metrics.mjs';

const PROCESS_FIELDS = [
  'process_id',
  'parent_process_id',
  'threads',
  'open_file_descriptors',
  'child_processes',
  'zombie_child_processes',
  'cgroup_pids_current',
  'cgroup_pids_max',
];

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  await main();
}

async function main() {
  const environment = required('SCOPE_RAILWAY_ENVIRONMENT');
  const since = process.env.SCOPE_RAILWAY_SINCE || '1h';
  const services = (process.env.SCOPE_RAILWAY_SERVICES || 'scope-api,scope-worker')
    .split(',')
    .map((service) => service.trim())
    .filter(Boolean);
  const logLines = positiveInteger('SCOPE_RAILWAY_LOG_LINES', 5_000);
  const outputRoot = resolve(process.env.SCOPE_RAILWAY_TELEMETRY_DIR || '.tmp/bench/railway-stress');
  const runId = new Date().toISOString().replaceAll(':', '-');
  await mkdir(outputRoot, { recursive: true });

  const report = {
    version: 2,
    generatedAt: new Date().toISOString(),
    runLabel: process.env.SCOPE_BENCH_RUN_LABEL?.trim() || 'unlabeled',
    environment,
    since,
    services: {},
  };
  for (const service of services) {
    const logs = parseJsonLines(await railway([
      'logs',
      '--service', service,
      '--environment', environment,
      '--since', since,
      '--lines', String(logLines),
      '--json',
    ]));
    const snapshots = logs.flatMap((entry) => {
      const message = stripAnsi(entry.message || '');
      if (!message.includes('runtime process snapshot')) return [];
      return [{ timestamp: entry.timestamp, ...numericFields(message, PROCESS_FIELDS) }];
    });
    const compactions = logs.flatMap((entry) => {
      const message = stripAnsi(entry.message || '');
      if (!message.includes('Git compaction attempt completed')) return [];
      return [{ timestamp: entry.timestamp, ...compactionFields(message) }];
    });
    const pushPersistence = logs.flatMap((entry) => {
      const message = stripAnsi(entry.message || '');
      if (!message.includes('Git push persistence timing')) return [];
      return [{ timestamp: entry.timestamp, ...pushPersistenceFields(message) }];
    });
    const objectStoreOperations = logs.flatMap((entry) => {
      const message = stripAnsi(entry.message || '');
      if (!message.includes('object store operation timing')) return [];
      return [{ timestamp: entry.timestamp, ...objectStoreFields(message) }];
    });
    const errors = logs.flatMap((entry) => {
      const message = stripAnsi(entry.message || '');
      return /Resource temporarily unavailable|capacity is exhausted|failed to spawn|panicked/i.test(message)
        ? [{ timestamp: entry.timestamp, message }]
        : [];
    });
    const metrics = JSON.parse(await railway([
      'metrics',
      '--service', service,
      '--environment', environment,
      '--since', since,
      '--raw',
      '--cpu',
      '--memory',
      '--json',
    ]));
    report.services[service] = {
      processSummary: summarizeSnapshots(snapshots),
      resourceSummary: summarizeMetrics(metrics),
      snapshots,
      compactions,
      compactionSummary: summarizeCompactions(compactions),
      pushPersistence,
      pushPersistenceSummary: summarizePushPersistence(pushPersistence),
      objectStoreOperations,
      objectStoreSummary: summarizeObjectStore(objectStoreOperations),
      errors,
    };
  }

  const output = join(outputRoot, `telemetry-${runId}.json`);
  const summary = join(outputRoot, `telemetry-${runId}.md`);
  await writeFile(output, `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(summary, telemetryMarkdown(report));
  console.log(`results: ${output}\nsummary: ${summary}`);
}

function railway(args) {
  return command('railway', args, {
    ...process.env,
    RAILWAY_CALLER: 'skill:use-railway@1.3.6',
    RAILWAY_AGENT_SESSION: process.env.RAILWAY_AGENT_SESSION || 'railway-scope-stress-collector',
  });
}

function command(program, args, env) {
  return new Promise((resolveCommand, rejectCommand) => {
    const child = spawn(program, args, { env, stdio: ['ignore', 'pipe', 'pipe'] });
    const stdout = [];
    const stderr = [];
    child.stdout.on('data', (chunk) => stdout.push(chunk));
    child.stderr.on('data', (chunk) => stderr.push(chunk));
    child.on('error', rejectCommand);
    child.on('close', (code) => {
      if (code === 0) resolveCommand(Buffer.concat(stdout).toString('utf8'));
      else rejectCommand(new Error(Buffer.concat(stderr).toString('utf8') || `${program} exited ${code}`));
    });
  });
}

export function stripAnsi(value) {
  return value.replaceAll(/\x1B\[[0-9;]*[mK]/g, '');
}

export function numericFields(message, names) {
  return Object.fromEntries(names.flatMap((name) => {
    const value = message.match(new RegExp(`\\b${name}=([0-9]+)`))?.[1];
    return value ? [[name, Number(value)]] : [];
  }));
}

export function compactionFields(message) {
  const numeric = numericFields(message, [
    'target_sequence',
    'scheduler_attempts',
    'scheduler_queue_delay_ms',
    'source_span_count',
    'source_pack_bytes',
    'predecessor_pack_bytes',
    'compacted_bytes',
    'candidate_query_ms',
    'init_ms',
    'download_ms',
    'index_ms',
    'update_ref_ms',
    'connectivity_check_ms',
    'pack_ms',
    'pack_total_ms',
    'store_ms',
    'persist_ms',
    'total_ms',
  ]);
  return {
    outcome: message.match(/\boutcome="?([^"\s]+)"?/)?.[1] || 'unknown',
    repoId: message.match(/\brepo_id=([^\s]+)/)?.[1] || null,
    ...numeric,
  };
}

export function summarizeCompactions(events) {
  return {
    count: events.length,
    outcomes: Object.fromEntries(groupBy(events, (event) => event.outcome).map(([outcome, values]) => [outcome, values.length])),
    queueDelayMs: timingSummary(events, 'scheduler_queue_delay_ms'),
    attempts: timingSummary(events, 'scheduler_attempts'),
    totalMs: timingSummary(events, 'total_ms'),
  };
}

export function pushPersistenceFields(message) {
  return {
    repositoryId: textField(message, 'repository_id'),
    protocol: textField(message, 'protocol') || 'unknown',
    ...numericFields(message, ['lock_wait_us', 'serialized_us', 'body_us', 'commit_us', 'total_us']),
  };
}

export function objectStoreFields(message) {
  return {
    operation: textField(message, 'operation') || 'unknown',
    success: booleanField(message, 'success'),
    ...numericFields(message, ['bytes', 'elapsed_us']),
  };
}

export function summarizePushPersistence(events) {
  return Object.fromEntries(groupBy(events, ({ protocol }) => protocol).map(([protocol, values]) => [protocol, {
    count: values.length,
    lockWaitUs: timingSummary(values, 'lock_wait_us'),
    serializedUs: timingSummary(values, 'serialized_us'),
    bodyUs: timingSummary(values, 'body_us'),
    commitUs: timingSummary(values, 'commit_us'),
    totalUs: timingSummary(values, 'total_us'),
  }]));
}

export function summarizeObjectStore(events) {
  return Object.fromEntries(groupBy(events, ({ operation }) => operation).map(([operation, values]) => {
    const successful = values.filter(({ success }) => success === true);
    const totalBytes = successful.reduce((sum, event) => sum + (event.bytes || 0), 0);
    const totalElapsedUs = successful.reduce((sum, event) => sum + (event.elapsed_us || 0), 0);
    return [operation, {
      count: values.length,
      failures: values.filter(({ success }) => success === false).length,
      elapsedUs: timingSummary(values, 'elapsed_us'),
      totalBytes,
      serviceTimeMiBPerSecond: totalElapsedUs > 0 ? round(totalBytes / 1024 / 1024 / (totalElapsedUs / 1_000_000)) : null,
    }];
  }));
}

export function summarizeSnapshots(snapshots) {
  return Object.fromEntries(PROCESS_FIELDS.flatMap((field) => {
    const values = snapshots.map((snapshot) => snapshot[field]).filter(Number.isFinite);
    return values.length > 0
      ? [[field, { minimum: Math.min(...values), maximum: Math.max(...values), last: values.at(-1) }]]
      : [];
  }));
}

function summarizeMetrics(metrics) {
  return Object.fromEntries(Object.entries(metrics.measurements || {}).map(([name, points]) => {
    const values = points.map(({ value }) => value).filter(Number.isFinite);
    return [name, values.length > 0 ? { minimum: Math.min(...values), maximum: Math.max(...values), last: values.at(-1) } : null];
  }));
}

function timingSummary(events, field) {
  const values = events.map((event) => event[field]).filter(Number.isFinite);
  return values.length ? {
    minimum: Math.min(...values),
    p50: percentile(values, 0.5),
    p95: percentile(values, 0.95),
    p99: percentile(values, 0.99),
    maximum: Math.max(...values),
  } : null;
}

function groupBy(values, keyFor) {
  const groups = new Map();
  for (const value of values) {
    const key = keyFor(value);
    const group = groups.get(key) || [];
    group.push(value);
    groups.set(key, group);
  }
  return [...groups.entries()].sort(([left], [right]) => left.localeCompare(right));
}

function textField(message, name) {
  return message.match(new RegExp(`\\b${name}=(?:"([^"]*)"|([^\\s]+))`))?.slice(1).find(Boolean) || null;
}

function booleanField(message, name) {
  const value = textField(message, name);
  return value === 'true' ? true : value === 'false' ? false : null;
}

function parseJsonLines(value) {
  return value.split('\n').map((line) => line.trim()).filter(Boolean).map(JSON.parse);
}

function telemetryMarkdown(report) {
  const compactionRows = Object.entries(report.services).map(([service, data]) => {
    const summary = data.compactionSummary;
    const outcomes = Object.entries(summary.outcomes).map(([name, count]) => `${name}:${count}`).join(', ') || 'none';
    return `| ${service} | ${summary.count} | ${outcomes} | ${summary.queueDelayMs?.p95 ?? 'n/a'} | ${summary.attempts?.maximum ?? 'n/a'} | ${summary.totalMs?.p95 ?? 'n/a'} |`;
  }).join('\n');
  const persistenceRows = Object.entries(report.services).flatMap(([service, data]) =>
    Object.entries(data.pushPersistenceSummary).map(([protocol, summary]) =>
      `| ${service} | ${protocol} | ${summary.count} | ${summary.lockWaitUs?.p95 ?? 'n/a'} | ${summary.bodyUs?.p95 ?? 'n/a'} | ${summary.commitUs?.p95 ?? 'n/a'} | ${summary.totalUs?.p95 ?? 'n/a'} |`,
    )).join('\n');
  const objectRows = Object.entries(report.services).flatMap(([service, data]) =>
    Object.entries(data.objectStoreSummary).map(([operation, summary]) =>
      `| ${service} | ${operation} | ${summary.count} | ${summary.failures} | ${summary.elapsedUs?.p95 ?? 'n/a'} | ${summary.totalBytes} | ${summary.serviceTimeMiBPerSecond ?? 'n/a'} |`,
    )).join('\n');
  return `# Railway Git storage telemetry\n\nGenerated: ${report.generatedAt}\n\nRun label: ${report.runLabel}\n\nEnvironment: ${report.environment}\n\n## Compaction scheduler\n\n| Service | Count | Outcomes | Queue delay p95 ms | Max attempts | Total p95 ms |\n|---|---:|---|---:|---:|---:|\n${compactionRows}\n\n## Push persistence\n\n| Service | Protocol | Count | Lock wait p95 us | Body p95 us | Commit p95 us | Total p95 us |\n|---|---|---:|---:|---:|---:|---:|\n${persistenceRows}\n\n## Object storage\n\n| Service | Operation | Count | Failures | Latency p95 us | Bytes | Service-time MiB/s |\n|---|---|---:|---:|---:|---:|---:|\n${objectRows}\n`;
}

function required(name) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required`);
  return value;
}

function positiveInteger(name, fallback) {
  const value = Number.parseInt(process.env[name] || String(fallback), 10);
  if (!Number.isInteger(value) || value < 1) throw new Error(`${name} must be a positive integer`);
  return value;
}
