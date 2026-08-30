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

const PUSH_PERSISTENCE_TIMINGS = [
  ['lockWaitUs', 'lock_wait_us'],
  ['metadataUs', 'metadata_us'],
  ['repositoryRowUs', 'repository_row_us'],
  ['hydrateUs', 'hydrate_us'],
  ['cloneUs', 'clone_us'],
  ['loadLiveFilesUs', 'load_live_files_us'],
  ['loadPreviousCommitUs', 'load_previous_commit_us'],
  ['loadGitHeadUs', 'load_git_head_us'],
  ['domainApplyUs', 'domain_apply_us'],
  ['catalogVerifyUs', 'catalog_verify_us'],
  ['repositoryFactsUs', 'repository_facts_us'],
  ['loadPackSpansUs', 'load_pack_spans_us'],
  ['historyRowsUs', 'history_rows_us'],
  ['liveFileRowsUs', 'live_file_rows_us'],
  ['saveDeltaUs', 'save_delta_us'],
  ['landingFileUs', 'landing_file_us'],
  ['workflowCatalogUs', 'workflow_catalog_us'],
  ['projectionUs', 'projection_us'],
  ['pushTriggerUs', 'push_trigger_us'],
  ['orphanQueueUs', 'orphan_queue_us'],
  ['serializedUs', 'serialized_us'],
  ['bodyUs', 'body_us'],
  ['commitUs', 'commit_us'],
  ['totalUs', 'total_us'],
];
const PUSH_PERSISTENCE_COUNTS = [
  'changed_file_count',
  'live_file_count',
  'logical_commit_count',
  'visibility_change_set_count',
  'policy_rule_count',
  'config_rule_count',
];

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  await main();
}

async function main() {
  const environment = required('SCOPE_RAILWAY_ENVIRONMENT');
  const since = process.env.SCOPE_RAILWAY_SINCE || '1h';
  const until = process.env.SCOPE_RAILWAY_UNTIL?.trim() || null;
  const services = (process.env.SCOPE_RAILWAY_SERVICES || 'scope-api,scope-worker')
    .split(',')
    .map((service) => service.trim())
    .filter(Boolean);
  const logLines = positiveInteger('SCOPE_RAILWAY_LOG_LINES', 5_000);
  const outputRoot = resolve(process.env.SCOPE_RAILWAY_TELEMETRY_DIR || '.tmp/bench/railway-stress');
  const runId = new Date().toISOString().replaceAll(':', '-');
  await mkdir(outputRoot, { recursive: true });

  const report = {
    version: 4,
    generatedAt: new Date().toISOString(),
    runLabel: process.env.SCOPE_BENCH_RUN_LABEL?.trim() || 'unlabeled',
    environment,
    since,
    until,
    services: {},
  };
  for (const service of services) {
    const [logs, restoreLogs, contentLogs, materializationLogs, segmentIngestLogs, segmentRestoreLogs] = await Promise.all([
      railwayLogs(service, environment, since, until, logLines),
      railwayLogs(service, environment, since, until, logLines, '"Git restore operation completed"'),
      railwayLogs(service, environment, since, until, logLines, '"Git content read completed"'),
      railwayLogs(service, environment, since, until, logLines, '"repository Git replica materialization completed"'),
      railwayLogs(service, environment, since, until, logLines, '"Git segment ingest telemetry"'),
      railwayLogs(service, environment, since, until, logLines, '"Git segment restore telemetry"'),
    ]);
    const gitLogs = [restoreLogs, contentLogs, materializationLogs].flat();
    const segmentLogs = [segmentIngestLogs, segmentRestoreLogs].flat();
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
      if (!isPushPersistenceMessage(message)) return [];
      return [{ timestamp: entry.timestamp, ...pushPersistenceFields(message) }];
    });
    const objectStoreOperations = logs.flatMap((entry) => {
      const message = stripAnsi(entry.message || '');
      if (!message.includes('object store operation timing')) return [];
      return [{ timestamp: entry.timestamp, ...objectStoreFields(message) }];
    });
    const gitOperations = gitLogs.flatMap((entry) => {
      const message = stripAnsi(entry.message || '');
      if (!message.includes('Git restore operation completed')
        && !message.includes('Git content read completed')
        && !message.includes('repository Git replica materialization completed')) return [];
      return [{ timestamp: entry.timestamp, ...gitOperationFields(message) }];
    });
    const gitSegmentTelemetry = segmentLogs.flatMap((entry) => {
      const event = gitSegmentTelemetryFields(stripAnsi(entry.message || ''));
      return event ? [{ timestamp: entry.timestamp, ...event }] : [];
    });
    const errors = logs.flatMap((entry) => {
      const message = stripAnsi(entry.message || '');
      return /Resource temporarily unavailable|capacity is exhausted|failed to spawn|panicked/i.test(message)
        ? [{ timestamp: entry.timestamp, message }]
        : [];
    });
    const capacityRejections = logs.flatMap((entry) => {
      const rejection = capacityRejectionFields(stripAnsi(entry.message || ''));
      return rejection ? [{ timestamp: entry.timestamp, ...rejection }] : [];
    });
    const metrics = JSON.parse(await railway(railwayMetricArgs(service, environment, since, until)));
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
      gitOperations,
      gitOperationSummary: summarizeGitOperations(gitOperations),
      materializationSummary: summarizeMaterializations(gitOperations),
      gitSegmentTelemetry,
      gitSegmentSummary: summarizeGitSegmentTelemetry(gitSegmentTelemetry),
      errors,
      capacityRejections,
      capacityRejectionSummary: summarizeCapacityRejections(capacityRejections),
    };
  }

  const output = join(outputRoot, `telemetry-${runId}.json`);
  const summary = join(outputRoot, `telemetry-${runId}.md`);
  await writeFile(output, `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(summary, telemetryMarkdown(report));
  console.log(`results: ${output}\nsummary: ${summary}`);
}

async function railwayLogs(service, environment, since, until, lines, filter = null) {
  const args = [
    'logs',
    '--service', service,
    '--environment', environment,
    '--since', since,
    '--lines', String(lines),
    '--json',
  ];
  if (until) args.push('--until', until);
  if (filter) args.push('--filter', filter);
  return parseJsonLines(await railway(args));
}

export function railwayMetricArgs(service, environment, since, until = null) {
  const args = [
    'metrics',
    '--service', service,
    '--environment', environment,
    '--since', since,
    '--raw',
    '--cpu',
    '--memory',
    '--json',
  ];
  if (until) args.push('--until', until);
  return args;
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

export function capacityRejectionFields(message) {
  if (message.includes('runtime capacity permit rejected')) {
    const operation = textField(message, 'operation');
    return operation ? { operation } : null;
  }
  const match = message.match(/(Git receive-pack|Git upload-pack|Git materialization|object store (?:read|write|delete)) capacity is exhausted/i);
  return match ? { operation: match[1] } : null;
}

export function summarizeCapacityRejections(events) {
  return Object.fromEntries(
    groupBy(events, (event) => event.operation).map(([operation, values]) => [operation, values.length]),
  );
}

export function pushPersistenceFields(message) {
  return {
    repositoryId: textField(message, 'repository_id'),
    protocol: textField(message, 'protocol') || 'unknown',
    configChanged: booleanField(message, 'config_changed'),
    ...numericFields(message, [
      ...PUSH_PERSISTENCE_COUNTS,
      ...PUSH_PERSISTENCE_TIMINGS.map(([, field]) => field),
    ]),
  };
}

export function isPushPersistenceMessage(message) {
  return message.includes('Git push persistence timing')
    || message.includes('repository mutation persistence timing');
}

export function objectStoreFields(message) {
  return {
    operation: textField(message, 'operation') || 'unknown',
    success: booleanField(message, 'success'),
    ...numericFields(message, ['bytes', 'elapsed_us']),
  };
}

export function gitOperationFields(message) {
  const numeric = numericFields(message, [
    'duration_ms',
    'elapsed_us',
    'requested_sequence',
    'applied_sequence_before',
    'applied_sequence_after',
    'pack_span_count',
    'span_index',
    'span_count',
    'first_sequence',
    'last_sequence',
    'geometric_tier',
    'size_bytes',
    'expected_size_bytes',
    'actual_size_bytes',
    'total_pack_bytes',
  ]);
  const operation = message.includes('repository Git replica materialization completed')
    ? 'materialize_repository'
    : textField(message, 'operation') || 'unknown';
  const sizeBytes = Number.isFinite(numeric.size_bytes)
    ? numeric.size_bytes
    : operation === 'cat_file' ? numeric.actual_size_bytes : null;
  return {
    requestId: textField(message, 'request_id'),
    replicaId: textField(message, 'replica_id'),
    repositoryId: textField(message, 'repository_id'),
    operation,
    cacheOutcome: textField(message, 'cache_outcome'),
    materializationPath: textField(message, 'materialization_path'),
    success: booleanField(message, 'success'),
    durationMs: Number.isFinite(numeric.duration_ms)
      ? numeric.duration_ms
      : Number.isFinite(numeric.elapsed_us) ? round(numeric.elapsed_us / 1_000) : null,
    ...numeric,
    ...(Number.isFinite(sizeBytes) ? { size_bytes: sizeBytes } : {}),
  };
}

export function gitSegmentTelemetryFields(message) {
  const kind = message.includes('Git segment ingest telemetry')
    ? 'ingest'
    : message.includes('Git segment restore telemetry') ? 'restore' : null;
  if (!kind) return null;
  return {
    kind,
    phase: textField(message, 'phase') || 'unknown',
    repositoryId: textField(message, 'repository_id'),
    segmentId: textField(message, 'segment_id'),
    success: booleanField(message, 'success'),
    ...numericFields(message, [
      'duration_us',
      'bytes',
      'blocked_us',
      'active_ingests',
      'buffered_bytes',
      'disk_free_bytes',
      'ledger_uploading',
      'ledger_ready',
      'ledger_published',
      'orphan_count',
    ]),
  };
}

export function summarizeGitSegmentTelemetry(events) {
  const phases = Object.fromEntries(groupBy(events, ({ kind, phase }) => `${kind}/${phase}`).map(([phase, values]) => [phase, {
    count: values.length,
    failures: values.filter(({ success }) => success === false).length,
    durationUs: timingSummary(values, 'duration_us'),
    blockedUs: timingSummary(values, 'blocked_us'),
    totalBytes: values.reduce((total, event) => total + (event.bytes || 0), 0),
  }]));
  return {
    phases,
    activeIngests: gaugeSummary(events, 'active_ingests'),
    bufferedBytes: gaugeSummary(events, 'buffered_bytes'),
    diskFreeBytes: gaugeSummary(events, 'disk_free_bytes'),
    ledgerUploading: gaugeSummary(events, 'ledger_uploading'),
    ledgerReady: gaugeSummary(events, 'ledger_ready'),
    ledgerPublished: gaugeSummary(events, 'ledger_published'),
    orphanCount: gaugeSummary(events, 'orphan_count'),
  };
}

export function summarizePushPersistence(events) {
  return Object.fromEntries(groupBy(events, ({ protocol }) => protocol).map(([protocol, values]) => [protocol, {
    count: values.length,
    configChanges: values.filter(({ configChanged }) => configChanged === true).length,
    changedFileCount: gaugeSummary(values, 'changed_file_count'),
    liveFileCount: gaugeSummary(values, 'live_file_count'),
    ...Object.fromEntries(PUSH_PERSISTENCE_TIMINGS.map(([name, field]) => [
      name,
      timingSummary(values, field),
    ])),
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

export function summarizeGitOperations(events) {
  return Object.fromEntries(groupBy(events, ({ operation }) => operation).map(([operation, values]) => [operation, {
    count: values.length,
    failures: values.filter(({ success }) => success === false).length,
    durationMs: timingSummary(values, 'durationMs'),
    totalDurationMs: round(values.reduce((total, event) => total + (event.durationMs || 0), 0)),
    totalBytes: values.reduce((total, event) => total + (event.size_bytes || 0), 0),
  }]));
}

export function summarizeMaterializations(events) {
  const materializations = events.filter(({ operation }) => operation === 'materialize_repository');
  return Object.fromEntries(groupBy(
    materializations,
    ({ cacheOutcome, materializationPath }) => `${cacheOutcome || 'unknown'}/${materializationPath || 'unknown'}`,
  ).map(([outcome, values]) => [outcome, {
    count: values.length,
    durationMs: timingSummary(values, 'durationMs'),
  }]));
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

function gaugeSummary(events, field) {
  const values = events.map((event) => event[field]).filter(Number.isFinite);
  return values.length ? {
    minimum: Math.min(...values),
    maximum: Math.max(...values),
    last: values.at(-1),
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
  return message.match(new RegExp(`\\b${name}=(?:"([^"]*)"|([^\\s},:]+))`))?.slice(1).find(Boolean) || null;
}

function booleanField(message, name) {
  const value = textField(message, name);
  return value === 'true' ? true : value === 'false' ? false : null;
}

function parseJsonLines(value) {
  return value.split('\n').map((line) => line.trim()).filter(Boolean).map(JSON.parse);
}

function telemetryMarkdown(report) {
  const segmentRows = Object.entries(report.services).flatMap(([service, data]) =>
    Object.entries(data.gitSegmentSummary.phases).map(([phase, summary]) =>
      `| ${service} | ${phase} | ${summary.count} | ${summary.failures} | ${summary.durationUs?.p95 ?? 'n/a'} | ${summary.blockedUs?.p95 ?? 'n/a'} | ${summary.totalBytes} |`,
    )).join('\n') || '| none | none | 0 | 0 | n/a | n/a | 0 |';
  const segmentPressureRows = Object.entries(report.services).map(([service, data]) => {
    const summary = data.gitSegmentSummary;
    return `| ${service} | ${summary.activeIngests?.maximum ?? 'n/a'} | ${summary.bufferedBytes?.maximum ?? 'n/a'} | ${summary.diskFreeBytes?.minimum ?? 'n/a'} | ${summary.ledgerUploading?.last ?? 'n/a'} | ${summary.ledgerReady?.last ?? 'n/a'} | ${summary.ledgerPublished?.last ?? 'n/a'} | ${summary.orphanCount?.maximum ?? 'n/a'} |`;
  }).join('\n');
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
  const gitOperationRows = Object.entries(report.services).flatMap(([service, data]) =>
    Object.entries(data.gitOperationSummary).map(([operation, summary]) =>
      `| ${service} | ${operation} | ${summary.count} | ${summary.failures} | ${summary.durationMs?.p95 ?? 'n/a'} | ${summary.totalDurationMs} | ${summary.totalBytes} |`,
    )).join('\n');
  const materializationRows = Object.entries(report.services).flatMap(([service, data]) =>
    Object.entries(data.materializationSummary).map(([outcome, summary]) =>
      `| ${service} | ${outcome} | ${summary.count} | ${summary.durationMs?.p95 ?? 'n/a'} |`,
    )).join('\n');
  const pressureRows = Object.entries(report.services).map(([service, data]) => {
    const process = data.processSummary;
    const resources = data.resourceSummary;
    const cpu = resources.CPU_USAGE?.maximum ?? 'n/a';
    const rssMiB = resources.MEMORY_USAGE_GB?.maximum == null
      ? 'n/a'
      : round(resources.MEMORY_USAGE_GB.maximum * 1024, 2);
    return `| ${service} | ${cpu} | ${rssMiB} | ${process.cgroup_pids_current?.maximum ?? 'n/a'} | ${process.open_file_descriptors?.maximum ?? 'n/a'} | ${process.zombie_child_processes?.maximum ?? 'n/a'} |`;
  }).join('\n');
  const rejectionRows = Object.entries(report.services).flatMap(([service, data]) =>
    Object.entries(data.capacityRejectionSummary).map(([operation, count]) =>
      `| ${service} | ${operation} | ${count} |`,
    )).join('\n') || '| none | none | 0 |';
  return `# Railway Git storage telemetry\n\nGenerated: ${report.generatedAt}\n\nRun label: ${report.runLabel}\n\nEnvironment: ${report.environment}\n\n## Git segment phases\n\n| Service | Kind/phase | Count | Failures | Duration p95 us | Blocked p95 us | Bytes |\n|---|---|---:|---:|---:|---:|---:|\n${segmentRows}\n\n## Git segment pressure and cleanup\n\n| Service | Peak active ingests | Peak buffered bytes | Minimum disk free bytes | Uploading last | Ready last | Published last | Peak orphans |\n|---|---:|---:|---:|---:|---:|---:|---:|\n${segmentPressureRows}\n\n## Git materialization outcomes\n\n| Service | Cache/path | Count | Duration p95 ms |\n|---|---|---:|---:|\n${materializationRows}\n\n## Git materialization phases\n\n| Service | Operation | Count | Failures | Duration p95 ms | Summed service ms | Bytes |\n|---|---|---:|---:|---:|---:|---:|\n${gitOperationRows}\n\n## Compaction scheduler\n\n| Service | Count | Outcomes | Queue delay p95 ms | Max attempts | Total p95 ms |\n|---|---:|---|---:|---:|---:|\n${compactionRows}\n\n## Capacity rejections\n\n| Service | Operation | Count |\n|---|---|---:|\n${rejectionRows}\n\n## Runtime pressure\n\n| Service | Peak CPU cores | Peak RSS MiB | Peak cgroup PIDs | Peak open FDs | Peak zombies |\n|---|---:|---:|---:|---:|---:|\n${pressureRows}\n\n## Push persistence\n\n| Service | Protocol | Count | Lock wait p95 us | Body p95 us | Commit p95 us | Total p95 us |\n|---|---|---:|---:|---:|---:|---:|\n${persistenceRows}\n\n## Object storage\n\n| Service | Operation | Count | Failures | Latency p95 us | Bytes | Service-time MiB/s |\n|---|---|---:|---:|---:|---:|---:|\n${objectRows}\n`;
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
