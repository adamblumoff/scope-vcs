import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoCodeRouteData } from './repo-code-route-data'
import { createRepoCodeRouteHandoff } from './repo-code-route-handoff'

const data = {
  content: Promise.resolve({ clone_remote_url: 'remote', files: [] }),
  selectedFile: null,
  selectedPath: null,
} satisfies RepoCodeRouteData

test('hands the default README result to its explicit URL once', () => {
  const handoff = createRepoCodeRouteHandoff()
  const repo = { owner: 'adam', repo: 'scope' }

  handoff.stage(repo, data)

  assert.equal(handoff.take({ ...repo, path: 'README.html' }), data)
  assert.equal(handoff.take({ ...repo, path: 'README.html' }), null)
})

test('does not reuse route data for another file or repository', () => {
  const handoff = createRepoCodeRouteHandoff()
  const repo = { owner: 'adam', repo: 'scope' }

  handoff.stage(repo, data)
  assert.equal(handoff.take({ ...repo, path: 'src/app.ts' }), null)

  handoff.stage(repo, data)
  assert.equal(handoff.take({ ...repo, repo: 'other' }), null)
})
