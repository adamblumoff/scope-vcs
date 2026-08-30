const TARGET_KINDS = new Set(['loadtest', 'staging']);

export function validateTargetKind(kind) {
  if (!TARGET_KINDS.has(kind)) {
    throw new Error('SCOPE_BENCH_TARGET_KIND must be loadtest or staging');
  }
}

export function assertSafeTarget(target, kind = 'loadtest') {
  validateTargetKind(kind);
  const url = new URL(target);
  const local = ['localhost', '127.0.0.1', '::1'].includes(url.hostname);
  const labels = url.hostname.toLowerCase().split('.').flatMap((label) => label.split('-'));
  if (!local && !labels.includes(kind)) {
    throw new Error(`refusing non-${kind} target: ${url.hostname}`);
  }
}
