import assert from 'node:assert/strict'
import test from 'node:test'
import { requestDiscussionThreadState } from './request-discussion-thread-state'
import type { RequestDiscussionView } from './request-discussion-types'

function discussion(
  overrides: Partial<RequestDiscussionView> = {},
): RequestDiscussionView {
  return {
    anchor: null,
    author: { handle: 'river', id: 'user_river' },
    body_markdown: 'body',
    client_discussion_id: 'client_1',
    created_at_unix: 0,
    id: 'discussion_1',
    last_activity_position: 1,
    latest_replies: [],
    opened_position: 1,
    reply_count: 0,
    request_id: 'request_1',
    resolved_at_unix: null,
    resolved_by: null,
    status: 'Open',
    unread_count: 0,
    ...overrides,
  }
}

test('a failed post outranks every other state', () => {
  assert.equal(
    requestDiscussionThreadState(
      discussion({ pending: 'failed', status: 'Resolved', unread_count: 4 }),
    ),
    'failed',
  )
})

test('resolved outranks unread', () => {
  assert.equal(
    requestDiscussionThreadState(
      discussion({ status: 'Resolved', unread_count: 4 }),
    ),
    'resolved',
  )
})

test('an open discussion with unread activity is unread', () => {
  assert.equal(
    requestDiscussionThreadState(discussion({ unread_count: 1 })),
    'unread',
  )
})

test('an open discussion with nothing new is read', () => {
  assert.equal(requestDiscussionThreadState(discussion()), 'read')
})
