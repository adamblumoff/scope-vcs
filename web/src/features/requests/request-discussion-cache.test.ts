import assert from 'node:assert/strict'
import test from 'node:test'
import { collectionFromPage } from './request-discussion-model'
import {
  readRequestDiscussionCache,
  readRequestDiscussionScroll,
  requestDiscussionCacheKey,
  resetRequestDiscussionCache,
  writeRequestDiscussionCache,
  writeRequestDiscussionScroll,
} from './request-discussion-cache'
import type { RequestDiscussion } from './request-discussion-types'

test('keys timeline views by viewer, repository, and request', () => {
  const base = {
    repoId: 'scope/demo',
    requestId: 'request-1',
    viewerId: 'user-maya',
  }
  assert.equal(
    requestDiscussionCacheKey(base),
    requestDiscussionCacheKey({ ...base }),
  )
  assert.notEqual(
    requestDiscussionCacheKey(base),
    requestDiscussionCacheKey({ ...base, requestId: 'request-2' }),
  )
  assert.notEqual(
    requestDiscussionCacheKey(base),
    requestDiscussionCacheKey({ ...base, viewerId: 'user-ravi' }),
  )
})

test('bounds cached views and preserves scroll with the entry', () => {
  resetRequestDiscussionCache()
  for (let index = 0; index < 10; index += 1) {
    const key = `request-${index}`
    writeRequestDiscussionCache(
      key,
      collectionFromPage({
        discussions: [discussion(key)],
        next_cursor: null,
        snapshot_version: 1,
      }),
    )
    writeRequestDiscussionScroll(key, index * 10)
  }
  assert.equal(readRequestDiscussionCache('request-0'), null)
  assert.equal(readRequestDiscussionScroll('request-9'), 90)
})

test('preserves per-discussion expansion metadata with cached view state', () => {
  resetRequestDiscussionCache()
  const key = 'expanded'
  const collection = collectionFromPage({
    discussions: [discussion('discussion-1')],
    next_cursor: null,
    snapshot_version: 1,
  })
  const expanded = collection.byId.get('discussion-1')
  assert.ok(expanded)
  writeRequestDiscussionCache(key, {
    ...collection,
    byId: new Map([
      ['discussion-1', { ...expanded, expanded: true }],
    ]),
  })
  assert.equal(
    readRequestDiscussionCache(key)?.byId.get('discussion-1')?.expanded,
    true,
  )
})

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
    request_id: id,
    resolved_at_unix: null,
    resolved_by: null,
    status: 'Open',
    unread_count: 0,
  }
}
