import type { RepoFileContent } from '@/api/types'

const MAX_CACHE_ENTRIES = 32
const MAX_CACHE_BYTES = 24 * 1024 * 1024

export type RepoFileCacheEntry = {
  approximateBytes: number
  file: RepoFileContent
  lastAccessed: number
}

const entries = new Map<string, RepoFileCacheEntry>()
let accessClock = 0
let totalBytes = 0

export function readRepoFileCache(key: string) {
  const entry = entries.get(key)
  if (!entry) return null
  entry.lastAccessed = nextAccess()
  return entry.file
}

export function writeRepoFileCache(key: string, file: RepoFileContent) {
  const previous = entries.get(key)
  if (previous) totalBytes -= previous.approximateBytes

  const entry = {
    approximateBytes: approximateFileBytes(file),
    file,
    lastAccessed: nextAccess(),
  }
  entries.set(key, entry)
  totalBytes += entry.approximateBytes
  evictOldEntries(key)
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
  totalBytes = 0
  accessClock = 0
}

export function repoFileCacheStats() {
  return { entries: entries.size, totalBytes }
}

function approximateFileBytes(file: RepoFileContent) {
  return file.content.kind === 'text'
    ? file.content.text.length * 2
    : file.content.size_bytes
}

function evictOldEntries(protectedKey: string) {
  while (entries.size > MAX_CACHE_ENTRIES || totalBytes > MAX_CACHE_BYTES) {
    let oldest: [string, RepoFileCacheEntry] | null = null
    for (const entry of entries) {
      if (entry[0] === protectedKey && entries.size > 1) continue
      if (!oldest || entry[1].lastAccessed < oldest[1].lastAccessed) {
        oldest = entry
      }
    }
    if (!oldest) return
    entries.delete(oldest[0])
    totalBytes -= oldest[1].approximateBytes
  }
}

function nextAccess() {
  accessClock += 1
  return accessClock
}
