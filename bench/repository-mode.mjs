const REPOSITORY_MODES = new Set(['spread', 'hot']);
const HOT_REPOSITORY_WORKLOADS = new Set(['warm-fetch', 'full-clone']);

export function validateRepositoryMode(mode, workloads, historyDepths, readReplicaCount) {
  if (!REPOSITORY_MODES.has(mode)) throw new Error(`SCOPE_LOAD_REPOSITORY_MODE must be one of ${[...REPOSITORY_MODES].join(', ')}`);
  if (mode !== 'hot') return;
  const unsupported = workloads.filter((workload) => !HOT_REPOSITORY_WORKLOADS.has(workload));
  if (unsupported.length) throw new Error(`hot repository mode supports only ${[...HOT_REPOSITORY_WORKLOADS].join(', ')}; received ${unsupported.join(', ')}`);
  if (historyDepths.length !== 1) {
    throw new Error('hot repository mode requires one SCOPE_LOAD_HISTORY_DEPTHS value');
  }
  if (!readReplicaCount?.trim()) throw new Error('hot repository mode requires SCOPE_LOAD_READ_REPLICA_COUNT');
}

export function fetchClientCount(config) {
  if (config.repositoryMode !== 'hot') return Math.max(config.mixedRepos, ...config.stages);
  return config.rates ? config.maxInFlight : Math.max(...config.stages);
}

export function needsFetchClients(workloads) {
  return workloads.some((workload) => ['warm-fetch', 'incremental-fetch'].includes(workload));
}
