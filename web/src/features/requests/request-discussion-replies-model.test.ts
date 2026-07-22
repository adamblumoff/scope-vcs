import assert from 'node:assert/strict'
import test from 'node:test'
import {
  acknowledgeReply,
  createDiscussionRepliesState,
  directDiscussionReplies,
  insertOptimisticReply,
  markReplyFailed,
  mergeDiscussionReplies,
  mergeReplyPage,
  replyTreeFullyExposed,
  updateReplyPage,
} from './request-discussion-replies-model'
import type { RequestDiscussionReplyView } from './request-discussion-types'

test('posting from a collapsed reply preview preserves existing replies', () => {
  const preview = reply('preview', 1)
  const optimistic = { ...reply('optimistic', 2), pending: 'sending' as const }
  const state = insertOptimisticReply(
    createDiscussionRepliesState(),
    optimistic,
    [preview],
  )

  assert.deepEqual(ids(state.replies), ['preview', 'optimistic'])
})

test('posting after loading older replies preserves chronological order', () => {
  const current = [reply('one', 1), reply('two', 2), reply('three', 3)]
  const latest = [reply('two', 2), reply('three', 3)]
  const state = insertOptimisticReply(
    createDiscussionRepliesState(current),
    { ...reply('four', 4), pending: 'sending' },
    latest,
  )

  assert.deepEqual(ids(state.replies), ['one', 'two', 'three', 'four'])
})

test('expanded replies merge new realtime previews in order', () => {
  const replies = mergeDiscussionReplies(
    [reply('one', 1), reply('two', 2)],
    [reply('two', 2), reply('three', 3)],
  )

  assert.deepEqual(ids(replies), ['one', 'two', 'three'])
})

test('reply trees expose only the requested direct children', () => {
  const root = reply('root', 1)
  const child = nestedReply('child', 2, root.id)
  const grandchild = nestedReply('grandchild', 3, child.id)
  const replies = [root, child, grandchild]

  assert.deepEqual(ids(directDiscussionReplies(replies, null)), ['root'])
  assert.deepEqual(ids(directDiscussionReplies(replies, root.id)), ['child'])
  assert.deepEqual(ids(directDiscussionReplies(replies, child.id)), [
    'grandchild',
  ])
})

test('root pages merge previews and track the exhausted cursor', () => {
  const initial = updateReplyPage(createDiscussionRepliesState(), null, {
    error: null,
    loading: true,
  })
  const loaded = mergeReplyPage(
    initial,
    null,
    { next_before_position: null, replies: [reply('older', 1)] },
    [reply('preview', 3)],
  )

  assert.deepEqual(ids(loaded.replies), ['older', 'preview'])
  assert.deepEqual(loaded.root, {
    error: null,
    loaded: true,
    loading: false,
    nextBeforePosition: null,
  })
})

test('failed page loads preserve their cursor and can restart', () => {
  const loaded = mergeReplyPage(
    createDiscussionRepliesState(),
    null,
    { next_before_position: 10, replies: [reply('newer', 11)] },
  )
  const loading = updateReplyPage(loaded, null, {
    error: null,
    loading: true,
  })
  const failed = updateReplyPage(loading, null, {
    error: 'Older replies could not be loaded.',
    loading: false,
  })
  const restarted = updateReplyPage(failed, null, {
    error: null,
    loading: true,
  })

  assert.equal(failed.root.nextBeforePosition, 10)
  assert.equal(failed.root.loading, false)
  assert.equal(failed.root.error, 'Older replies could not be loaded.')
  assert.equal(restarted.root.loading, true)
  assert.equal(restarted.root.error, null)
})

test('page updates clear errors without disturbing pagination state', () => {
  const loaded = mergeReplyPage(
    createDiscussionRepliesState(),
    null,
    { next_before_position: 10, replies: [reply('newer', 11)] },
  )
  const failed = updateReplyPage(loaded, null, {
    error: 'Replies could not be loaded.',
  })
  const cleared = updateReplyPage(failed, null, { error: null })

  assert.deepEqual(cleared.root, { ...failed.root, error: null })
})

test('branch pages merge children and record the known child count', () => {
  const parent = { ...reply('parent', 1), child_reply_count: 3 }
  const loaded = mergeReplyPage(
    createDiscussionRepliesState([parent]),
    parent.id,
    {
      next_before_position: 2,
      replies: [nestedReply('child', 3, parent.id)],
    },
  )

  assert.deepEqual(ids(loaded.replies), ['parent', 'child'])
  assert.deepEqual(loaded.branches.get(parent.id), {
    error: null,
    knownChildCount: 3,
    loaded: true,
    loading: false,
    nextBeforePosition: 2,
  })
})

test('optimistic nested replies increment their parent exactly once', () => {
  const parent = reply('parent', 1)
  const optimistic = {
    ...nestedReply('client-child', Number.MAX_SAFE_INTEGER, parent.id),
    pending: 'sending' as const,
  }
  const inserted = insertOptimisticReply(
    createDiscussionRepliesState([parent]),
    optimistic,
  )
  const duplicate = insertOptimisticReply(
    inserted,
    optimistic,
    [parent],
  )

  assert.equal(find(inserted, parent.id).child_reply_count, 1)
  assert.equal(find(duplicate, parent.id).child_reply_count, 1)
  assert.deepEqual(ids(duplicate.replies), ['parent', 'client-child'])
})

test('failure, retry, and acknowledgment preserve the parent count', () => {
  const parent = reply('parent', 1)
  const optimistic = {
    ...nestedReply('client-child', Number.MAX_SAFE_INTEGER, parent.id),
    pending: 'sending' as const,
  }
  const inserted = insertOptimisticReply(
    createDiscussionRepliesState([parent]),
    optimistic,
  )
  const failed = updateReplyPage(
    markReplyFailed(inserted, optimistic.id),
    null,
    { error: 'Reply could not be posted.' },
  )
  const retried = insertOptimisticReply(failed, optimistic)
  const acknowledged = acknowledgeReply(
    retried,
    optimistic.id,
    nestedReply('server-child', 2, parent.id),
  )

  assert.equal(find(failed, optimistic.id).pending, 'failed')
  assert.equal(find(retried, optimistic.id).pending, 'sending')
  assert.equal(retried.root.error, null)
  assert.equal(find(acknowledged, parent.id).child_reply_count, 1)
  assert.equal(find(acknowledged, 'server-child').pending, undefined)
  assert.deepEqual(ids(acknowledged.replies), ['parent', 'server-child'])
})

test('the full tree requires every root and branch cursor to end', () => {
  const parent = { ...reply('parent', 1), child_reply_count: 1 }
  const child = nestedReply('child', 2, parent.id)
  const rootPaged = mergeReplyPage(
    createDiscussionRepliesState(),
    null,
    { next_before_position: 1, replies: [parent, child] },
  )
  const rootLoaded = mergeReplyPage(
    rootPaged,
    null,
    { next_before_position: null, replies: [parent, child] },
  )
  const branchPaged = mergeReplyPage(rootLoaded, parent.id, {
    next_before_position: 1,
    replies: [child],
  })
  const branchExhausted = mergeReplyPage(branchPaged, parent.id, {
    next_before_position: null,
    replies: [child],
  })

  assert.equal(fullyExposed(rootPaged, new Set([parent.id]), 2), false)
  assert.equal(fullyExposed(rootLoaded, new Set(), 2), false)
  assert.equal(fullyExposed(branchPaged, new Set([parent.id]), 2), false)
  assert.equal(fullyExposed(branchExhausted, new Set([parent.id]), 2), true)
})

test('complete nested previews can establish root exposure explicitly', () => {
  const parent = { ...reply('parent', 1), child_reply_count: 1 }
  const child = nestedReply('child', 2, parent.id)
  const previewState = mergeReplyPage(
    createDiscussionRepliesState([parent, child]),
    parent.id,
    {
      next_before_position: null,
      replies: [child],
    },
  )

  assert.equal(
    replyTreeFullyExposed({
      expandedReplyIds: new Set([parent.id]),
      replyCount: 2,
      rootRepliesLoaded: true,
      state: previewState,
    }),
    true,
  )
})

function fullyExposed(
  state: ReturnType<typeof createDiscussionRepliesState>,
  expandedReplyIds: ReadonlySet<string>,
  replyCount: number,
) {
  return replyTreeFullyExposed({
    expandedReplyIds,
    replyCount,
    rootRepliesLoaded: state.root.loaded,
    state,
  })
}

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

function nestedReply(id: string, position: number, parentId: string) {
  return { ...reply(id, position), reply_to_reply_id: parentId }
}

function reply(id: string, position: number): RequestDiscussionReplyView {
  return {
    author: { handle: 'maya', id: 'user-maya' },
    body_markdown: `Reply ${id}`,
    child_reply_count: 0,
    can_reply: true,
    created_at_unix: position,
    discussion_id: 'one',
    id,
    position,
    reply_to_reply_id: null,
  }
}
