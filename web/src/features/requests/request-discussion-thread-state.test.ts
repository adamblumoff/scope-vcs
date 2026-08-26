import assert from 'node:assert/strict'
import test from 'node:test'
import {
  requestDiscussionThreadState,
  showsThreadReplyToggle,
} from './request-discussion-thread-state'
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
    root_reply_count: 0,
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

test('a thread whose only root reply is already shown gets no toggle', () => {
  // ten replies in the tree, one of them root-level and already previewed:
  // expanding the root list would reveal nothing
  assert.equal(
    showsThreadReplyToggle({
      expanded: false,
      rootReplyCount: 1,
      visibleCount: 1,
    }),
    false,
  )
})

test('a thread with unshown root replies gets a toggle', () => {
  assert.equal(
    showsThreadReplyToggle({
      expanded: false,
      rootReplyCount: 3,
      visibleCount: 1,
    }),
    true,
  )
})

test('an expanded thread keeps its toggle so it can collapse', () => {
  assert.equal(
    showsThreadReplyToggle({
      expanded: true,
      rootReplyCount: 1,
      visibleCount: 1,
    }),
    true,
  )
})
