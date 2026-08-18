import type { RepoFileContent } from '@/api/types'
import { createBoundedCache } from '../../lib/bounded-cache'

const MAX_CACHE_ENTRIES = 32
const MAX_CACHE_BYTES = 24 * 1024 * 1024

const entries = createBoundedCache<string, RepoFileContent>({
  maxEntries: MAX_CACHE_ENTRIES,
  maxWeight: MAX_CACHE_BYTES,
  weightOf: approximateFileBytes,
})

export function readRepoFileCache(key: string) {
  return entries.get(key) ?? null
}

export function peekRepoFileCache(key: string) {
  return entries.peek(key) ?? null
}

export function writeRepoFileCache(key: string, file: RepoFileContent) {
  entries.set(key, file)
}

export function repoFileCacheKey({
  audience,
  changeVersion,
  oid,
  path,
  repoId,
}: {
  audience: 'private' | 'public'
  changeVersion: number
  oid: string
  path: string
  repoId: string
}) {
  return [repoId, changeVersion, audience, path, oid].join('\0')
}

export function resetRepoFileCache() {
  entries.clear()
}

export function repoFileCacheStats() {
  const stats = entries.stats()
  return { entries: stats.entries, totalBytes: stats.totalWeight }
}

function approximateFileBytes(file: RepoFileContent) {
  return file.content.kind === 'text'
    ? file.content.text.length * 2
    : file.content.size_bytes
}
