export const ROUTING_MODES = new Set(['single', 'random', 'repository-affine']);

export function parseApiUrls(primary, value) {
  const candidates = [primary, ...(value || '').split(',')]
    .map((entry) => entry.trim().replace(/\/$/, ''))
    .filter(Boolean);
  return [...new Set(candidates)];
}

export function createEndpointRouter(apiUrls, mode, seed = 1) {
  if (!Array.isArray(apiUrls) || apiUrls.length === 0) throw new Error('at least one API URL is required');
  if (!ROUTING_MODES.has(mode)) throw new Error(`unsupported routing mode: ${mode}`);
  let randomState = seed >>> 0;
  return {
    choose(repositoryKey = '') {
      if (mode === 'single' || apiUrls.length === 1) return apiUrls[0];
      if (mode === 'repository-affine') return apiUrls[stableHash(repositoryKey) % apiUrls.length];
      randomState = (Math.imul(1_664_525, randomState) + 1_013_904_223) >>> 0;
      return apiUrls[randomState % apiUrls.length];
    },
  };
}

function stableHash(value) {
  let hash = 2_166_136_261;
  for (const byte of Buffer.from(value)) {
    hash ^= byte;
    hash = Math.imul(hash, 16_777_619) >>> 0;
  }
  return hash;
}
