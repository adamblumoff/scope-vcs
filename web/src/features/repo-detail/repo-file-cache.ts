import type { RepoFileContent } from '@/api/types'
import { createBoundedCache } from '../../lib/bounded-cache'

const entries = createBoundedCache<string, RepoFileContent>({
  maxEntries: 32,
  maxWeight: 24 * 1024 * 1024,
  weightOf: (file) => file.content.kind === 'text'
    ? file.content.text.length * 2
    : file.content.size_bytes,
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
  path,
  repoId,
}: {
  audience: 'private' | 'public'
  changeVersion: number
  path: string
  repoId: string
}) {
  return [repoId, changeVersion, audience, path.replace(/^\/+/, '')].join('\0')
}
