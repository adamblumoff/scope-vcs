import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import { parseRepoChangeEvent, takeSseMessages } from './repo-live-refresh'

test('parseRepoChangeEvent reads repo change SSE payloads', () => {
  assert.deepEqual(
    parseRepoChangeEvent(
      'event: repo-change\ndata: {"repo_id":"owner/repo","version":2,"reason":"visibility-changed"}',
    ),
    {
      reason: 'visibility-changed',
      repo_id: 'owner/repo',
      version: 2,
    },
  )
})

test('parseRepoChangeEvent ignores keepalive comments', () => {
  assert.equal(parseRepoChangeEvent(': keep-alive'), null)
})

test('takeSseMessages keeps partial message buffered', () => {
  assert.deepEqual(takeSseMessages('event: one\n\nevent: two'), {
    messages: ['event: one'],
    rest: 'event: two',
  })
})
