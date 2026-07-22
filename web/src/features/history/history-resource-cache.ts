import type {
  CommitDetail,
  ProjectionPreviewAudience,
  ReviewFileDiff,
} from '@/api/types'
import { createBoundedCache } from '../../lib/bounded-cache'

const MAX_COMMIT_ENTRIES = 48
const MAX_COMMIT_BYTES = 4 * 1024 * 1024
const MAX_DIFF_ENTRIES = 20
const MAX_DIFF_BYTES = 32 * 1024 * 1024

type DiffCacheEntry = {
  scrollTop: number
  value: ReviewFileDiff
}

const commitEntries = createBoundedCache<string, CommitDetail>({
  maxEntries: MAX_COMMIT_ENTRIES,
  maxWeight: MAX_COMMIT_BYTES,
  weightOf: approximateSerializedBytes,
})
const diffEntries = createBoundedCache<string, DiffCacheEntry>({
  maxEntries: MAX_DIFF_ENTRIES,
  maxWeight: MAX_DIFF_BYTES,
  weightOf: (entry) => approximateSerializedBytes(entry.value),
})

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
  return commitEntries.get(key) ?? null
}

export function peekHistoryCommitCache(key: string) {
  return commitEntries.peek(key) ?? null
}

export function writeHistoryCommitCache(key: string, value: CommitDetail) {
  commitEntries.set(key, value)
}

export function readHistoryDiffCache(key: string) {
  return diffEntries.get(key)?.value ?? null
}

export function peekHistoryDiffCache(key: string) {
  return diffEntries.peek(key)?.value ?? null
}

export function writeHistoryDiffCache(key: string, value: ReviewFileDiff) {
  const scrollTop = diffEntries.peek(key)?.scrollTop ?? 0
  diffEntries.set(key, { scrollTop, value })
}

export function readHistoryDiffScroll(key: string | null) {
  if (!key) return 0
  return diffEntries.peek(key)?.scrollTop ?? 0
}

export function writeHistoryDiffScroll(key: string | null, scrollTop: number) {
  if (!key) return
  const entry = diffEntries.peek(key)
  if (entry) entry.scrollTop = scrollTop
}

export function resetHistoryResourceCache() {
  commitEntries.clear()
  diffEntries.clear()
}

export function historyResourceCacheStats() {
  const commits = commitEntries.stats()
  const diffs = diffEntries.stats()
  return {
    commitBytes: commits.totalWeight,
    commits: commits.entries,
    diffBytes: diffs.totalWeight,
    diffs: diffs.entries,
  }
}

function approximateSerializedBytes(value: unknown) {
  return JSON.stringify(value).length * 2
}
