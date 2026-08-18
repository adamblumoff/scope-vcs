import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoContent, RepoFileContent } from '@/api/types'
import {
  DEFAULT_REPO_FILE_PATH,
  loadRepoCodeRouteData,
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

const content: RepoContent = {
  clone_remote_url: 'https://example.com/owner/repo.git',
  files: [{
    oid: readme.oid,
    path: readme.path,
    tracked: true,
    visibility: readme.visibility,
  }],
}

test('starts the tree before awaiting the primary file and leaves it deferred', async () => {
  const calls: string[] = []
  let resolveContent: (value: RepoContent) => void = () => undefined
  const contentPromise = new Promise<RepoContent>((resolve) => {
    resolveContent = resolve
  })

  const result = await loadRepoCodeRouteData({
    loadContent: () => {
      calls.push('content')
      return contentPromise
    },
    loadFile: async (path) => {
      calls.push(`file:${path}`)
      return readme
    },
  })

  assert.deepEqual(calls, ['content', `file:${DEFAULT_REPO_FILE_PATH}`])
  assert.equal(result.content, contentPromise)
  assert.equal(result.requestedPath, DEFAULT_REPO_FILE_PATH)
  assert.equal(result.selectedFile, readme)
  assert.equal(result.selectedPath, readme.path)

  resolveContent(content)
  assert.equal(await result.content, content)
})

test('does not open files that are absent from the projection', async () => {
  const loadContent = async () => content
  const loadFile = async () => null

  const explicit = await loadRepoCodeRouteData({
    loadContent,
    loadFile,
    requestedPath: 'missing.txt',
  })
  const defaulted = await loadRepoCodeRouteData({ loadContent, loadFile })

  assert.equal(explicit.selectedPath, null)
  assert.equal(explicit.requestedPath, 'missing.txt')
  assert.equal(defaulted.selectedPath, null)
})

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
