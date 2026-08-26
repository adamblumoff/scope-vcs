import assert from 'node:assert/strict'
import test from 'node:test'
import {
  acknowledgeReply,
  beforePositionForNextReplyPage,
  countVisibleUnreadReplies,
  createDiscussionRepliesState,
  hasLoadedAllUnreadContent,
  insertOptimisticReply,
  markReplyFailed,
  mergeDiscussionReplies,
  mergeReplyPage,
  mergeReplyTarget,
  updateReplyPage,
} from './request-discussion-replies-model'
import type { RequestDiscussionReplyView } from './request-discussion-types'

test('posting from a preview preserves existing replies', () => {
  const preview = reply('preview', 1)
  const optimistic = { ...reply('optimistic', 2), pending: 'sending' as const }
  const state = insertOptimisticReply(
    createDiscussionRepliesState(),
    optimistic,
    [preview],
  )

  assert.deepEqual(ids(state.replies), ['preview', 'optimistic'])
})

test('posting after loading earlier replies preserves chronological order', () => {
  const current = [reply('one', 1), reply('two', 2), reply('three', 3)]
  const latest = [reply('two', 2), reply('three', 3)]
  const state = insertOptimisticReply(
    createDiscussionRepliesState(current),
    { ...reply('four', 4), pending: 'sending' },
    latest,
  )

  assert.deepEqual(ids(state.replies), ['one', 'two', 'three', 'four'])
})

test('flat pages merge realtime previews without duplicates', () => {
  const state = mergeReplyPage(
    createDiscussionRepliesState([reply('three', 3)]),
    {
      next_before_position: null,
      replies: [reply('one', 1), reply('two', 2)],
    },
    [reply('three', 3), reply('four', 4)],
  )

  assert.deepEqual(ids(state.replies), ['one', 'two', 'three', 'four'])
  assert.deepEqual(state.page, {
    error: null,
    loaded: true,
    loading: false,
    nextBeforePosition: null,
    newestLoadedPosition: null,
  })
})

test('new preview activity refreshes the newest page before older pagination', () => {
  const loaded = mergeReplyPage(
    createDiscussionRepliesState(),
    {
      next_before_position: 51,
      replies: [reply('fifty-one', 51), reply('one-hundred', 100)],
    },
    [],
    true,
  )

  assert.equal(
    beforePositionForNextReplyPage(loaded, [
      reply('one-hundred-two', 102),
      reply('one-hundred-three', 103),
      reply('one-hundred-four', 104),
    ]),
    undefined,
  )

  const refreshed = mergeReplyPage(
    loaded,
    {
      next_before_position: 55,
      replies: [reply('fifty-five', 55), reply('one-hundred-four', 104)],
    },
    [],
    true,
  )
  assert.equal(
    beforePositionForNextReplyPage(refreshed, [
      reply('one-hundred-four', 104),
    ]),
    55,
  )
})

test('an exhausted page refreshes from the newest edge when preview activity advances', () => {
  const exhausted = mergeReplyPage(
    createDiscussionRepliesState(),
    {
      next_before_position: null,
      replies: [reply('one', 1), reply('three', 3)],
    },
    [],
    true,
  )

  assert.equal(
    beforePositionForNextReplyPage(exhausted, [
      reply('five', 5),
      reply('six', 6),
      reply('seven', 7),
    ]),
    undefined,
  )
})

test('fragment resolution merges one exact target without changing pagination', () => {
  const loaded = mergeReplyPage(
    createDiscussionRepliesState(),
    {
      next_before_position: 51,
      replies: [reply('newest', 100)],
    },
    [],
    true,
  )
  const resolved = mergeReplyTarget(loaded, {
    next_before_position: null,
    replies: [reply('target', 25)],
  })

  assert.deepEqual(ids(resolved.replies), ['target', 'newest'])
  assert.equal(resolved.page.nextBeforePosition, 51)
  assert.equal(resolved.page.newestLoadedPosition, 100)
})

test('failed page loads preserve their cursor and can restart', () => {
  const loaded = mergeReplyPage(createDiscussionRepliesState(), {
    next_before_position: 10,
    replies: [reply('newer', 11)],
  })
  const loading = updateReplyPage(loaded, { error: null, loading: true })
  const failed = updateReplyPage(loading, {
    error: 'Earlier replies could not be loaded.',
    loading: false,
  })
  const restarted = updateReplyPage(failed, { error: null, loading: true })

  assert.equal(failed.page.nextBeforePosition, 10)
  assert.equal(failed.page.loading, false)
  assert.equal(failed.page.error, 'Earlier replies could not be loaded.')
  assert.equal(restarted.page.loading, true)
  assert.equal(restarted.page.error, null)
})

test('reply references remain attached while pages merge', () => {
  const parent = reply('parent', 1)
  const child = referencedReply('child', 2, parent)
  const state = mergeReplyPage(createDiscussionRepliesState([child]), {
    next_before_position: null,
    replies: [parent],
  })

  assert.deepEqual(find(state, child.id).reply_to, {
    author: parent.author,
    body_markdown: parent.body_markdown,
    id: parent.id,
    position: parent.position,
  })
})

test('an eight-message reply chain remains one chronological collection', () => {
  const chain = [reply('one', 1)]
  for (let position = 2; position <= 8; position += 1) {
    chain.push(
      referencedReply(String(position), position, chain.at(-1)!),
    )
  }

  const state = mergeReplyPage(createDiscussionRepliesState(chain.slice(-3)), {
    next_before_position: null,
    replies: chain.slice(0, -3).reverse(),
  })

  assert.deepEqual(ids(state.replies), [
    'one',
    '2',
    '3',
    '4',
    '5',
    '6',
    '7',
    '8',
  ])
  assert.deepEqual(
    state.replies.slice(1).map((reply) => reply.reply_to?.id),
    ['one', '2', '3', '4', '5', '6', '7'],
  )
})

test('failure, retry insertion, and acknowledgment replace one optimistic row', () => {
  const parent = reply('parent', 1)
  const optimistic = {
    ...referencedReply('client-child', Number.MAX_SAFE_INTEGER, parent),
    pending: 'sending' as const,
  }
  const inserted = insertOptimisticReply(
    createDiscussionRepliesState([parent]),
    optimistic,
  )
  const failed = updateReplyPage(markReplyFailed(inserted, optimistic.id), {
    error: 'Reply could not be posted.',
  })
  const retried = insertOptimisticReply(failed, optimistic)
  const acknowledged = acknowledgeReply(
    retried,
    optimistic.id,
    referencedReply('server-child', 2, parent),
  )

  assert.equal(find(failed, optimistic.id).pending, 'failed')
  assert.equal(find(retried, optimistic.id).pending, 'sending')
  assert.equal(retried.page.error, null)
  assert.equal(find(acknowledged, 'server-child').pending, undefined)
  assert.deepEqual(ids(acknowledged.replies), ['parent', 'server-child'])
})

test('newer projections replace stale reply data', () => {
  const replies = mergeDiscussionReplies(
    [reply('one', 1, 'Old body')],
    [reply('one', 1, 'New body')],
  )

  assert.equal(replies[0]?.body_markdown, 'New body')
})

test('counts only loaded replies beyond the read position', () => {
  assert.equal(
    countVisibleUnreadReplies(
      [
        reply('read', 10),
        reply('new-one', 11),
        reply('new-two', 12),
        { ...reply('pending', Number.MAX_SAFE_INTEGER), pending: 'sending' },
      ],
      10,
    ),
    2,
  )
})

test('unread content is incomplete while earlier unread replies are hidden', () => {
  const visible = [
    reply('new-three', 13),
    reply('new-four', 14),
    reply('new-five', 15),
  ]

  assert.equal(hasLoadedAllUnreadContent(visible, 10, 5, false), false)
  assert.equal(hasLoadedAllUnreadContent(visible, 10, 3, false), true)
  assert.equal(hasLoadedAllUnreadContent(visible, 10, 4, true), true)
})

function ids(replies: RequestDiscussionReplyView[]) {
  return replies.map(({ id }) => id)
}

function find(
  state: ReturnType<typeof createDiscussionRepliesState>,
  id: string,
) {
  const found = state.replies.find((reply) => reply.id === id)
  assert.ok(found)
  return found
}

function referencedReply(
  id: string,
  position: number,
  parent: RequestDiscussionReplyView,
): RequestDiscussionReplyView {
  return {
    ...reply(id, position),
    reply_to: {
      author: parent.author,
      body_markdown: parent.body_markdown,
      id: parent.id,
      position: parent.position,
    },
  }
}

function reply(
  id: string,
  position: number,
  body = `Reply ${id}`,
): RequestDiscussionReplyView {
  return {
    author: { handle: 'maya', id: 'user-maya' },
    body_markdown: body,
    created_at_unix: position,
    discussion_id: 'one',
    id,
    position,
    reply_to: null,
  }
}
