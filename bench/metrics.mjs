const MIB = 1024 * 1024;

export function round(value) {
  return Number.isFinite(value) ? Math.round(value * 100) / 100 : null;
}

export function percentile(values, point) {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((left, right) => left - right);
  return round(sorted[Math.max(0, Math.ceil(sorted.length * point) - 1)]);
}

export function sampleStats(samples) {
  const durations = samples.map(({ durationMs }) => durationMs);
  const ttfb = samples.map(({ ttfbMs, durationMs }) => ttfbMs ?? durationMs);
  const schedule = samples.map(({ scheduleDelayMs = 0 }) => scheduleDelayMs);
  const successful = samples.filter(({ ok }) => ok);
  const sum = (field) => successful.reduce((total, sample) => total + (sample[field] || 0), 0);
  return {
    count: samples.length,
    ok: successful.length,
    meanMs: round(durations.reduce((total, value) => total + value, 0) / Math.max(1, durations.length)),
    p50Ms: percentile(durations, 0.5),
    p95Ms: percentile(durations, 0.95),
    p99Ms: percentile(durations, 0.99),
    ttfbP50Ms: percentile(ttfb, 0.5),
    ttfbP95Ms: percentile(ttfb, 0.95),
    ttfbP99Ms: percentile(ttfb, 0.99),
    scheduleDelayP95Ms: percentile(schedule, 0.95),
    bytes: sum('bytes'),
    logicalBytes: sum('logicalBytes'),
  };
}

export function normalizedRates(samples, elapsedSeconds) {
  const successful = samples.filter(({ ok }) => ok);
  const bytes = successful.reduce((total, sample) => total + (sample.bytes || 0), 0);
  const logicalBytes = successful.reduce((total, sample) => total + (sample.logicalBytes || 0), 0);
  return {
    operationsPerSecond: round(successful.length / elapsedSeconds),
    observedMiBPerSecond: round(bytes / MIB / elapsedSeconds),
    logicalMiBPerSecond: round(logicalBytes / MIB / elapsedSeconds),
    observedBytesPerOperation: round(bytes / Math.max(1, successful.length)),
    logicalBytesPerOperation: round(logicalBytes / Math.max(1, successful.length)),
  };
}

export function normalizeProcessMeasurement(measurement, inputBytes, outputBytes) {
  const inputMiB = inputBytes / MIB;
  const wallSeconds = measurement.wallMs / 1000;
  return {
    inputBytes,
    outputBytes,
    wallMsPerMiB: inputMiB > 0 ? round(measurement.wallMs / inputMiB) : null,
    cpuMsPerMiB: inputMiB > 0 ? round(measurement.cpuMs / inputMiB) : null,
    inputMiBPerSecond: wallSeconds > 0 ? round(inputMiB / wallSeconds) : null,
    outputToInputRatio: inputBytes > 0 ? round(outputBytes / inputBytes) : null,
  };
}

export function bytesLabel(bytes) {
  if (!Number.isFinite(bytes)) return 'n/a';
  if (bytes >= 1024 ** 3) return `${round(bytes / 1024 ** 3)} GiB`;
  if (bytes >= MIB) return `${round(bytes / MIB)} MiB`;
  if (bytes >= 1024) return `${round(bytes / 1024)} KiB`;
  return `${bytes} B`;
}
