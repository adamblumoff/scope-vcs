import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoFileContent } from '@/api/types'
import {
  loadRepoFileWhenReady,
  type RepoFileLoadResult,
} from './repo-code-route-data'

const readme: RepoFileContent = {
  content: { kind: 'text', text: '<h1>Scope</h1>' },
  oid: 'readme-oid',
  path: '/README.html',
  size_bytes: 14,
  visibility: 'Public',
}

test('retries a rebuilding projection before returning the primary file', async () => {
  const results: RepoFileLoadResult[] = [
    { status: 'rebuilding' },
    { status: 'rebuilding' },
    { file: readme, status: 'ready' },
  ]
  let attempts = 0

  const file = await loadRepoFileWhenReady({
    load: async () => {
      attempts += 1
      return results.shift() ?? { status: 'rebuilding' }
    },
    retryDelays: [0, 0, 0],
    signal: new AbortController().signal,
  })

  assert.equal(file, readme)
  assert.equal(attempts, 3)
})

test('stops retrying a rebuilding projection when navigation is aborted', async () => {
  const controller = new AbortController()
  controller.abort()
  let attempts = 0

  await assert.rejects(
    loadRepoFileWhenReady({
      load: async () => {
        attempts += 1
        return { status: 'rebuilding' }
      },
      retryDelays: [0],
      signal: controller.signal,
    }),
    { name: 'AbortError' },
  )
  assert.equal(attempts, 0)
})
