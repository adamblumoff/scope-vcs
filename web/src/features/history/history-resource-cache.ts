import type {
  CommitDetail,
  ProjectionPreviewAudience,
  ReviewFileDiff,
} from '@/api/types'

const MAX_COMMIT_ENTRIES = 48
const MAX_COMMIT_BYTES = 4 * 1024 * 1024
const MAX_DIFF_ENTRIES = 20
const MAX_DIFF_BYTES = 32 * 1024 * 1024

type CacheEntry<T> = {
  approximateBytes: number
  lastAccessed: number
  value: T
}

type DiffCacheEntry = CacheEntry<ReviewFileDiff> & {
  scrollTop: number
}

const commitEntries = new Map<string, CacheEntry<CommitDetail>>()
const diffEntries = new Map<string, DiffCacheEntry>()
let accessClock = 0
let commitBytes = 0
let diffBytes = 0

export function historyCommitCacheKey({
  audience,
  commit,
  generation,
  repoId,
  viewKey,
}: {
  audience: ProjectionPreviewAudience
  commit: string
  generation: string
  repoId: string
  viewKey: string
}) {
  return [repoId, generation, viewKey, audience, commit].join('\0')
}

export function historyDiffCacheKey({
  audience,
  commit,
  generation,
  newOid,
  oldOid,
  path,
  repoId,
  viewKey,
}: {
  audience: ProjectionPreviewAudience
  commit: string
  generation: string
  newOid: string | null
  oldOid: string | null
  path: string
  repoId: string
  viewKey: string
}) {
  return [
    repoId,
    generation,
    viewKey,
    audience,
    commit,
    path,
    oldOid ?? '',
    newOid ?? '',
  ].join('\0')
}

export function readHistoryCommitCache(key: string) {
  const entry = commitEntries.get(key)
  if (!entry) return null
  entry.lastAccessed = nextAccess()
  return entry.value
}

export function writeHistoryCommitCache(key: string, value: CommitDetail) {
  const previous = commitEntries.get(key)
  if (previous) commitBytes -= previous.approximateBytes
  const entry = createEntry(value)
  commitEntries.set(key, entry)
  commitBytes += entry.approximateBytes
  commitBytes = evictOldEntries(
    commitEntries,
    commitBytes,
    MAX_COMMIT_ENTRIES,
    MAX_COMMIT_BYTES,
    key,
  )
}

export function readHistoryDiffCache(key: string) {
  const entry = diffEntries.get(key)
  if (!entry) return null
  entry.lastAccessed = nextAccess()
  return entry.value
}

export function writeHistoryDiffCache(key: string, value: ReviewFileDiff) {
  const previous = diffEntries.get(key)
  if (previous) diffBytes -= previous.approximateBytes
  const entry = {
    ...createEntry(value),
    scrollTop: previous?.scrollTop ?? 0,
  }
  diffEntries.set(key, entry)
  diffBytes += entry.approximateBytes
  diffBytes = evictOldEntries(
    diffEntries,
    diffBytes,
    MAX_DIFF_ENTRIES,
    MAX_DIFF_BYTES,
    key,
  )
}

export function readHistoryDiffScroll(key: string | null) {
  if (!key) return 0
  return diffEntries.get(key)?.scrollTop ?? 0
}

export function writeHistoryDiffScroll(key: string | null, scrollTop: number) {
  if (!key) return
  const entry = diffEntries.get(key)
  if (entry) entry.scrollTop = scrollTop
}

export function resetHistoryResourceCache() {
  commitEntries.clear()
  diffEntries.clear()
  commitBytes = 0
  diffBytes = 0
  accessClock = 0
}

export function historyResourceCacheStats() {
  return {
    commitBytes,
    commits: commitEntries.size,
    diffBytes,
    diffs: diffEntries.size,
  }
}

function createEntry<T>(value: T): CacheEntry<T> {
  return {
    approximateBytes: JSON.stringify(value).length * 2,
    lastAccessed: nextAccess(),
    value,
  }
}

function evictOldEntries<T extends CacheEntry<unknown>>(
  entries: Map<string, T>,
  totalBytes: number,
  maxEntries: number,
  maxBytes: number,
  protectedKey: string,
) {
  while (entries.size > maxEntries || totalBytes > maxBytes) {
    let oldest: [string, T] | null = null
    for (const entry of entries) {
      if (entry[0] === protectedKey && entries.size > 1) continue
      if (!oldest || entry[1].lastAccessed < oldest[1].lastAccessed) {
        oldest = entry
      }
    }
    if (!oldest) break
    entries.delete(oldest[0])
    totalBytes -= oldest[1].approximateBytes
  }
  return totalBytes
}

function nextAccess() {
  accessClock += 1
  return accessClock
}
