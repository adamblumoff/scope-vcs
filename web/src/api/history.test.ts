import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import { parseCommitHistoryInput } from './history-inputs'

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
