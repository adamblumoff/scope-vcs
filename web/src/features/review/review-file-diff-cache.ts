import type { ReviewFileDiff } from '@/api/types'
import { createBoundedCache } from '../../lib/bounded-cache'
import type { FileDiffMetadata } from '@pierre/diffs'

const MAX_PARSED_DIFF_ENTRIES = 12
const MAX_PARSED_DIFF_BYTES = 16 * 1024 * 1024

type ParsedDiffEntry = {
  approximateBytes: number
  value: FileDiffMetadata
}

const parsedDiffs = createBoundedCache<string, ParsedDiffEntry>({
  maxEntries: MAX_PARSED_DIFF_ENTRIES,
  maxWeight: MAX_PARSED_DIFF_BYTES,
  weightOf: (entry) => entry.approximateBytes,
})

export function parsedDiffForReviewFile(
  diff: ReviewFileDiff,
  cacheKey: string | null | undefined,
  parse: (diff: ReviewFileDiff) => FileDiffMetadata | null,
): FileDiffMetadata | null {
  if (!cacheKey) return parse(diff)

  const cached = parsedDiffs.get(cacheKey)
  if (cached) return cached.value

  const value = parse(diff)
  if (!value) return null

  parsedDiffs.set(cacheKey, {
    approximateBytes: JSON.stringify(diff).length * 2,
    value,
  })
  return value
}

export function resetParsedDiffCache() {
  parsedDiffs.clear()
}

export function parsedDiffCacheStats() {
  const stats = parsedDiffs.stats()
  return { entries: stats.entries, totalBytes: stats.totalWeight }
}
