import assert from 'node:assert/strict'
import test from 'node:test'
import {
  replyFragment,
  replyTargetFromFragment,
  sameUtcDate,
  shouldGroupReplies,
} from './request-discussion-reply-presentation'
import type { RequestDiscussionReplyView } from './request-discussion-types'

test('groups consecutive messages from one author for five minutes', () => {
  const previous = reply(100)

  assert.equal(shouldGroupReplies(previous, reply(400), noBoundary), true)
  assert.equal(shouldGroupReplies(previous, reply(401), noBoundary), false)
  assert.equal(
    shouldGroupReplies(previous, { ...reply(200), author: otherAuthor }, noBoundary),
    false,
  )
})

test('date, unread, and pending states start a fresh message group', () => {
  const previous = reply(100)

  assert.equal(shouldGroupReplies(previous, reply(101), { date: true, unread: false }), false)
  assert.equal(shouldGroupReplies(previous, reply(101), { date: false, unread: true }), false)
  assert.equal(
    shouldGroupReplies(previous, { ...reply(101), pending: 'sending' }, noBoundary),
    false,
  )
})

test('date boundaries use a deterministic UTC calendar day', () => {
  assert.equal(sameUtcDate(86_399, 86_400), false)
  assert.equal(sameUtcDate(86_400, 86_401), true)
})

test('reply fragments identify one discussion and reply', () => {
  const fragment = replyFragment('discussion/a', 'reply #1')

  assert.equal(
    fragment,
    '#discussion=discussion%2Fa&reply=reply+%231',
  )
  assert.deepEqual(replyTargetFromFragment(fragment), {
    discussionId: 'discussion/a',
    replyId: 'reply #1',
  })
  assert.equal(replyTargetFromFragment('#discussion=one'), null)
  assert.equal(replyTargetFromFragment('discussion=one&reply=two'), null)
})

const noBoundary = { date: false, unread: false }
const otherAuthor = { handle: 'ravi', id: 'user-ravi' }

function reply(createdAtUnix: number): RequestDiscussionReplyView {
  return {
    author: { handle: 'maya', id: 'user-maya' },
    body_markdown: 'Reply',
    created_at_unix: createdAtUnix,
    discussion_id: 'discussion',
    id: `reply-${createdAtUnix}`,
    position: createdAtUnix,
    reply_to: null,
  }
}
