import type { RepoContent } from '@/api/types'
import { createBoundedCache } from '../../lib/bounded-cache'

const entries = createBoundedCache<string, RepoContent>({
  maxEntries: 8,
  maxWeight: 8 * 1024 * 1024,
  weightOf: approximateContentBytes,
})

export function readRepoContentCache(key: string) {
  return entries.get(key) ?? null
}

export function peekRepoContentCache(key: string) {
  return entries.peek(key) ?? null
}

export function writeRepoContentCache(key: string, content: RepoContent) {
  entries.set(key, content)
}

export function repoContentCacheKey({
  audience,
  changeVersion,
  repoId,
}: {
  audience: 'private' | 'public'
  changeVersion: number
  repoId: string
}) {
  return [repoId, changeVersion, audience].join('\0')
}

export function resetRepoContentCache() {
  entries.clear()
}

export function repoContentCacheStats() {
  return entries.stats()
}

function approximateContentBytes(content: RepoContent) {
  return content.clone_remote_url.length * 2 + content.files.reduce(
    (bytes, file) => bytes + file.path.length * 2 + file.oid.length * 2 + 32,
    0,
  )
}
