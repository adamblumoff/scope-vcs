import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import { parseCommitHistoryInput } from './history-inputs'
import { ApiRouteTemplates, buildApiPath } from './types.generated'

test('generated API paths encode every dynamic segment', () => {
  assert.equal(
    buildApiPath(ApiRouteTemplates.repoCommit, {
      owner: 'an owner',
      repo: 'r/name',
      commit_id: 'ref?#1',
    }),
    '/v1/repos/an%20owner/r%2Fname/commits/ref%3F%231',
  )
})

test('parseCommitHistoryInput rejects stale owner audience values', () => {
  assert.throws(
    () =>
      parseCommitHistoryInput({
        audience: 'owner',
        owner: 'adam',
        repo: 'scope',
      }),
    /Unsupported commit history audience/,
  )
})

test('parseCommitHistoryInput defaults missing audience to public', () => {
  assert.deepEqual(parseCommitHistoryInput({ owner: 'adam', repo: 'scope' }), {
    audience: 'public',
    owner: 'adam',
    repo: 'scope',
  })
})
