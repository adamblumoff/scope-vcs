import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoFileContent } from '@/api/types'
import {
  readRepoFileCache,
  repoFileCacheKey,
  repoFileCacheStats,
  resetRepoFileCache,
  writeRepoFileCache,
} from './repo-file-cache'

function textFile(path: string, oid: string, text: string): RepoFileContent {
  return {
    content: { kind: 'text', text },
    oid,
    path,
    size_bytes: text.length,
    visibility: 'Public',
  }
}

test('keys file entries by generation, audience, path, and oid', () => {
  const base = {
    audience: 'public' as const,
    changeVersion: 3,
    oid: 'abc',
    path: 'README.md',
    repoId: 'repo-1',
  }
  assert.notEqual(
    repoFileCacheKey(base),
    repoFileCacheKey({ ...base, audience: 'private' }),
  )
  assert.notEqual(
    repoFileCacheKey(base),
    repoFileCacheKey({ ...base, changeVersion: 4 }),
  )
  assert.notEqual(
    repoFileCacheKey(base),
    repoFileCacheKey({ ...base, oid: 'def' }),
  )
})

test('returns loaded files and evicts old entries at the entry limit', () => {
  resetRepoFileCache()
  for (let index = 0; index < 40; index += 1) {
    writeRepoFileCache(`file-${index}`, textFile(`${index}.ts`, `${index}`, 'x'))
  }

  assert.equal(repoFileCacheStats().entries, 32)
  assert.equal(readRepoFileCache('file-0'), null)
  assert.equal(readRepoFileCache('file-39')?.path, '39.ts')
})

test('uses a byte budget for large source entries', () => {
  resetRepoFileCache()
  const sixMiBOfText = 'x'.repeat(3 * 1024 * 1024)
  for (let index = 0; index < 6; index += 1) {
    writeRepoFileCache(
      `large-${index}`,
      textFile(`${index}.txt`, `${index}`, sixMiBOfText),
    )
  }

  const stats = repoFileCacheStats()
  assert.ok(stats.entries < 6)
  assert.ok(stats.totalBytes <= 24 * 1024 * 1024)
})
