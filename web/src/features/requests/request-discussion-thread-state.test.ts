import assert from 'node:assert/strict'
import test from 'node:test'
import { requestDiscussionRailColor } from './request-discussion-thread-state'

test('thread rail color follows failed, resolved, unread, read precedence', () => {
  assert.equal(
    requestDiscussionRailColor({
      pending: 'failed',
      status: 'Resolved',
      unread_count: 4,
    }),
    'bg-danger-border',
  )
  assert.equal(
    requestDiscussionRailColor({ status: 'Resolved', unread_count: 4 }),
    'bg-success-border',
  )
  assert.equal(
    requestDiscussionRailColor({ status: 'Open', unread_count: 1 }),
    'bg-brand',
  )
  assert.equal(
    requestDiscussionRailColor({ status: 'Open', unread_count: 0 }),
    'bg-border',
  )
})
