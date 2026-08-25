import type { RepoRunAttempt, RepoRunCache } from '@/api/types'

export function summarizeAttemptCaches(caches: readonly RepoRunCache[]) {
  let warm = 0
  let cold = 0
  let unavailable = 0
  for (const cache of caches) {
    const observation = cache.observation
    if (!observation) {
      unavailable += 1
      continue
    }
    if (observation.preparation.kind === 'cold') cold += 1
    else warm += 1
  }
  return { cold, unavailable, warm }
}

export function cacheSummaryLabel(
  caches: readonly RepoRunCache[],
  cacheSetup: RepoRunAttempt['cache_setup'],
) {
  if (caches.length === 0) return 'No caches declared'
  const summary = summarizeAttemptCaches(caches)
  const parts = [
    countLabel(summary.warm, 'warm'),
    countLabel(summary.cold, 'cold'),
    countLabel(summary.unavailable, 'not reported'),
  ].filter(Boolean)
  if (cacheSetup) {
    parts.push(`setup in ${formatMilliseconds(cacheSetup.wall_ms)}`)
    parts.push(`authorized in ${formatMilliseconds(cacheSetup.authorization_ms)}`)
  }
  return parts.join(' · ')
}

export function cacheStateLabel(cache: RepoRunCache) {
  const preparation = cache.observation?.preparation
  if (!preparation) return 'not reported'
  return preparation.kind
}

export function cacheStateClass(cache: RepoRunCache) {
  switch (cacheStateLabel(cache)) {
    case 'exact':
    case 'compatible':
      return 'text-success'
    case 'cold':
      return 'text-warning'
    default:
      return 'text-muted-foreground'
  }
}

export function cacheExplanation(cache: RepoRunCache) {
  const observation = cache.observation
  if (!observation) return 'Cache facts were not reported for this attempt.'
  if (observation.preparation.kind === 'exact') {
    return `Exact entry found · ${observation.final_state}`
  }
  if (observation.preparation.kind === 'compatible') {
    return `Compatible fallback found · ${observation.final_state}`
  }
  return `${coldReasonLabel(observation.preparation.reason)} · ${observation.final_state}`
}

export function cacheNamespace(cache: RepoRunCache) {
  const observation = cache.observation
  return observation
    ? `${observation.workflow_path} · ${observation.job_key}`
    : cache.path
}

export function cacheTimingLabel(cache: RepoRunCache) {
  const observation = cache.observation
  if (!observation) return 'unavailable'
  const prepare = `total ${formatMilliseconds(observation.prepare_ms)}`
  return observation.finalize_ms === null
    ? prepare
    : `${prepare} · finalize ${formatMilliseconds(observation.finalize_ms)}`
}

export function cachePreparationDetail(cache: RepoRunCache) {
  const observation = cache.observation
  if (!observation) return null
  return [
    `${formatBytes(observation.size_bytes)} compressed`,
    `key ${formatMilliseconds(observation.key_ms)}`,
    `metadata ${formatMilliseconds(observation.metadata_ms)}`,
    `download + verify ${formatMilliseconds(observation.download_verify_ms)}`,
    `sync ${formatMilliseconds(observation.sync_ms)}`,
    `extract ${formatMilliseconds(observation.extraction_ms)}`,
  ].join(' · ')
}

export function pinnedImageLabel(image: string | null) {
  if (!image) return 'Image not pinned yet'
  const digest = image.includes('@sha256:')
    ? image.split('@sha256:').pop() ?? image
    : image
  return `sha256:${digest.slice(0, 12)}`
}

function coldReasonLabel(reason: string) {
  switch (reason) {
    case 'metadata-missing':
      return 'No reusable entry for this identity'
    case 'metadata-invalid':
      return 'Cache metadata was invalid'
    case 'metadata-not-ready':
      return 'Cached volume was not ready'
    case 'volume-missing':
      return 'Cached volume was missing'
    case 'volume-invalid':
      return 'Cached volume was invalid'
    case 'backing-directory-missing':
      return 'Cache backing directory was missing'
    default:
      return 'Cache was cold'
  }
}

function countLabel(count: number, label: string) {
  return count === 0 ? null : `${count} ${label}`
}

function formatMilliseconds(milliseconds: number) {
  if (milliseconds < 1_000) return `${milliseconds}ms`
  return `${(milliseconds / 1_000).toFixed(1)}s`
}

function formatBytes(bytes: number) {
  if (bytes < 1_024) return `${bytes} B`
  if (bytes < 1_024 ** 2) return `${(bytes / 1_024).toFixed(1)} KiB`
  if (bytes < 1_024 ** 3) return `${(bytes / 1_024 ** 2).toFixed(1)} MiB`
  return `${(bytes / 1_024 ** 3).toFixed(2)} GiB`
}
