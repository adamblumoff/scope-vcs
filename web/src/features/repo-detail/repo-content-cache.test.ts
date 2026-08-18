import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoContent } from '@/api/types'
import {
  readRepoContentCache,
  repoContentCacheKey,
  repoContentCacheStats,
  resetRepoContentCache,
  writeRepoContentCache,
} from './repo-content-cache'

const content: RepoContent = {
  clone_remote_url: 'https://example.com/owner/repo.git',
  files: [],
}

test('keys repository content by version and audience', () => {
  const base = {
    audience: 'public' as const,
    changeVersion: 3,
    repoId: 'repo-1',
  }

  assert.notEqual(
    repoContentCacheKey(base),
    repoContentCacheKey({ ...base, audience: 'private' }),
  )
  assert.notEqual(
    repoContentCacheKey(base),
    repoContentCacheKey({ ...base, changeVersion: 4 }),
  )
})

test('bounds repository content entries', () => {
  resetRepoContentCache()
  for (let index = 0; index < 10; index += 1) {
    writeRepoContentCache(`repo-${index}`, content)
  }

  assert.equal(repoContentCacheStats().entries, 8)
  assert.equal(readRepoContentCache('repo-0'), null)
  assert.equal(readRepoContentCache('repo-9'), content)
})
