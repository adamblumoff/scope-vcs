import assert from 'node:assert/strict'
import test from 'node:test'
import { collectionFromPage } from './request-discussion-model'
import {
  readRequestDiscussionCache,
  readRequestDiscussionScroll,
  resetRequestDiscussionCache,
  writeRequestDiscussionCache,
  writeRequestDiscussionScroll,
} from './request-discussion-cache'
import type { RequestDiscussion } from './request-discussion-types'

test('preserves scroll when replacing a cached collection', () => {
  resetRequestDiscussionCache()
  writeRequestDiscussionCache('request', collection(['first']))
  writeRequestDiscussionScroll('request', 240)
  writeRequestDiscussionCache('request', collection(['second']))

  assert.equal(readRequestDiscussionScroll('request'), 240)
})

test('retains only the newest 500 discussion IDs', () => {
  resetRequestDiscussionCache()
  const ids = Array.from({ length: 501 }, (_, index) => `discussion-${index}`)
  writeRequestDiscussionCache('request', collection(ids))

  const cached = readRequestDiscussionCache('request')
  assert.ok(cached)
  assert.equal(cached.order.length, 500)
  assert.equal(cached.byId.has('discussion-500'), false)
  assert.equal(cached.byId.has('discussion-0'), true)
})

function collection(ids: string[]) {
  return collectionFromPage({
    discussions: ids.map(discussion),
    next_cursor: null,
    snapshot_version: 1,
  })
}

function discussion(id: string): RequestDiscussion {
  return {
    author: { handle: 'maya', id: 'user-maya' },
    body_markdown: id,
    client_discussion_id: id,
    created_at_unix: 1,
    id,
    last_activity_position: 1,
    latest_replies: [],
    opened_position: 1,
    reply_count: 0,
    request_id: 'request',
    resolved_at_unix: null,
    resolved_by: null,
    status: 'Open',
    unread_count: 0,
  }
}
