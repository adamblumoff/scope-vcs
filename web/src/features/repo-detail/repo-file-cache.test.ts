import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoFileContent } from '@/api/types'
import {
  peekRepoFileCache,
  readRepoFileCache,
  repoFileCacheKey,
  writeRepoFileCache,
} from './repo-file-cache'

function textFile(path: string, oid: string, text = 'x'): RepoFileContent {
  return {
    content: { kind: 'text', text },
    oid,
    path,
    size_bytes: text.length,
    visibility: 'Public',
  }
}

test('keys file entries by repository generation, audience, and path', () => {
  const base = {
    audience: 'public' as const,
    changeVersion: 3,
    path: '/README.html',
    repoId: 'repo-1',
  }

  assert.equal(repoFileCacheKey(base), repoFileCacheKey({ ...base, path: 'README.html' }))
  assert.notEqual(repoFileCacheKey(base), repoFileCacheKey({ ...base, audience: 'private' }))
  assert.notEqual(repoFileCacheKey(base), repoFileCacheKey({ ...base, changeVersion: 4 }))
  assert.notEqual(repoFileCacheKey(base), repoFileCacheKey({ ...base, path: '/src/app.ts' }))
})

test('keeps recently read files and evicts older entries at the entry limit', () => {
  for (let index = 0; index < 33; index += 1) {
    writeRepoFileCache(`file-${index}`, textFile(`/${index}.ts`, `${index}`))
  }

  assert.equal(readRepoFileCache('file-0'), null)
  assert.equal(peekRepoFileCache('file-32')?.path, '/32.ts')
})
