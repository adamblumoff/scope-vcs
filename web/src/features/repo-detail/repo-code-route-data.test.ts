import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoFileContent } from '@/api/types'
import {
  loadRepoFileWhenReady,
  repoCodeResourceLoader,
  settleRepoCodeResource,
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

test('addressed content resolves while the tree remains pending and retries reload', async () => {
  let finishTree: (() => void) | undefined
  let treeReady = false
  const tree = settleRepoCodeResource(new Promise<void>((resolve) => {
    finishTree = () => { treeReady = true; resolve() }
  }))
  let reloads = 0
  const load = repoCodeResourceLoader(settleRepoCodeResource(Promise.resolve(readme)), async () => {
    reloads += 1
    return readme
  })
  const signal = new AbortController().signal
  assert.equal(await load(signal), readme)
  assert.equal(treeReady, false)
  assert.equal(reloads, 0)
  assert.equal(await load(signal), readme)
  assert.equal(reloads, 1)
  finishTree?.()
  await tree
})

test('keeps missing-file errors local and allows a real retry after route failure', async () => {
  const load = repoCodeResourceLoader(
    settleRepoCodeResource(Promise.reject(new Error('File unavailable'))),
    async () => readme,
  )
  const signal = new AbortController().signal
  await assert.rejects(load(signal), { message: 'File unavailable' })
  assert.equal(await load(signal), readme)
})

test('does not publish deferred content after navigation cancellation', async () => {
  let finish: ((file: RepoFileContent) => void) | undefined
  const pending = new Promise<RepoFileContent>((resolve) => { finish = resolve })
  const load = repoCodeResourceLoader(settleRepoCodeResource(pending), async () => readme)
  const controller = new AbortController()
  const result = load(controller.signal)
  controller.abort()
  finish?.(readme)
  await assert.rejects(result, { name: 'AbortError' })
})

test('loads current access scope when previous route data no longer matches', async () => {
  let reloads = 0
  const load = repoCodeResourceLoader(null, async () => { reloads += 1; return readme })
  assert.equal(await load(new AbortController().signal), readme)
  assert.equal(reloads, 1)
})
