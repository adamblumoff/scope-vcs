import assert from 'node:assert/strict'
import test from 'node:test'
import {
  appendDiscussionPage,
  applyDiscussionChangesWithoutReordering,
  collectionFromPage,
  compactDiscussionSummary,
  insertOptimisticDiscussion,
  markDiscussionRead,
  mergeDiscussionReplies,
  patchDiscussionWithoutReordering,
  replaceDiscussion,
  upsertDiscussionReply,
} from './request-discussion-model'
import type {
  RequestDiscussion,
  RequestDiscussionView,
} from './request-discussion-types'

test('appends cursor pages without duplicating discussions', () => {
  const first = collectionFromPage({
    discussions: [discussion('one', 5), discussion('two', 4)],
    next_cursor: 'next',
    snapshot_version: 5,
  })
  const result = appendDiscussionPage(first, {
    discussions: [discussion('two', 4), discussion('three', 3)],
    next_cursor: null,
    snapshot_version: 5,
  })
  assert.deepEqual(result.order, ['three', 'two', 'one'])
  assert.equal(result.nextCursor, null)
})

test('realtime patches a discussion without moving it under the cursor', () => {
  const collection = collectionFromPage({
    discussions: [discussion('one', 5), discussion('two', 4)],
    next_cursor: null,
    snapshot_version: 5,
  })
  const patched = applyDiscussionChangesWithoutReordering(
    collection,
    [discussion('two', 9)],
    9,
  )
  assert.deepEqual(patched.order, ['two', 'one'])
})

test('realtime appends a new root without reordering visible roots', () => {
  const collection = collectionFromPage({
    discussions: [discussion('one', 5), discussion('two', 4)],
    next_cursor: null,
    snapshot_version: 5,
  })
  const patched = applyDiscussionChangesWithoutReordering(
    collection,
    [discussion('three', 8)],
    8,
  )
  assert.deepEqual(patched.order, ['two', 'one', 'three'])
  assert.equal(patched.snapshotVersion, 8)
})

test('realtime inserts an unseen older root by its opened position', () => {
  const collection = collectionFromPage({
    discussions: [discussion('one', 5), discussion('two', 4)],
    next_cursor: 'older',
    snapshot_version: 5,
  })
  const older = {
    ...discussion('older', 9),
    opened_position: 3,
  }
  const patched = applyDiscussionChangesWithoutReordering(
    collection,
    [older],
    9,
  )
  assert.deepEqual(patched.order, ['older', 'two', 'one'])
})

test('timeline keeps newly discovered resolved roots', () => {
  const collection = collectionFromPage({
    discussions: [discussion('one', 5)],
    next_cursor: null,
    snapshot_version: 5,
  })
  const resolved = {
    ...discussion('resolved', 8),
    resolved_at_unix: 8,
    status: 'Resolved' as const,
  }
  const patched = applyDiscussionChangesWithoutReordering(
    collection,
    [resolved],
    8,
  )
  assert.deepEqual(patched.order, ['one', 'resolved'])
  assert.equal(patched.snapshotVersion, 8)
})

test('timeline keeps roots in place when they become resolved', () => {
  const collection = collectionFromPage({
    discussions: [discussion('one', 5), discussion('two', 4)],
    next_cursor: null,
    snapshot_version: 5,
  })
  const resolved = {
    ...discussion('one', 8),
    resolved_at_unix: 8,
    status: 'Resolved' as const,
  }
  const patched = applyDiscussionChangesWithoutReordering(
    collection,
    [resolved],
    8,
  )
  assert.deepEqual(patched.order, ['two', 'one'])
  assert.equal(patched.byId.get('one')?.status, 'Resolved')
  assert.equal(patched.snapshotVersion, 8)
})

test('realtime timeline keeps roots that become resolved', () => {
  const collection = collectionFromPage({
    discussions: [discussion('one', 5)],
    next_cursor: null,
    snapshot_version: 5,
  })
  const resolved = {
    ...discussion('one', 8),
    resolved_at_unix: 8,
    status: 'Resolved' as const,
  }
  const patched = applyDiscussionChangesWithoutReordering(
    collection,
    [resolved],
    8,
  )
  assert.deepEqual(patched.order, ['one'])
  assert.equal(patched.byId.get('one')?.status, 'Resolved')
})

test('optimistic roots are replaced in their visible position', () => {
  const collection = collectionFromPage({
    discussions: [discussion('one', 5)],
    next_cursor: null,
    snapshot_version: 5,
  })
  const optimistic = {
    ...discussion('client-1', 6),
    pending: 'sending',
  } satisfies RequestDiscussionView
  const pending = insertOptimisticDiscussion(collection, optimistic)
  assert.deepEqual(pending.order, ['one', 'client-1'])
})

test('mutation responses do not advance the authoritative catch-up cursor', () => {
  const collection = collectionFromPage({
    discussions: [discussion('one', 5)],
    next_cursor: null,
    snapshot_version: 5,
  })
  const patched = replaceDiscussion(collection, discussion('one', 9))
  assert.equal(patched.snapshotVersion, 5)
  assert.equal(patched.byId.get('one')?.last_activity_position, 9)
})

test('mark read is monotonic in the client projection', () => {
  const collection = collectionFromPage({
    discussions: [{ ...discussion('one', 5), unread_count: 3 }],
    next_cursor: null,
    snapshot_version: 5,
  })
  const read = markDiscussionRead(collection, 'one')
  assert.equal(read.byId.get('one')?.unread_count, 0)
  assert.equal(markDiscussionRead(read, 'one'), read)
})

test('compact summary uses the first nonempty Markdown line', () => {
  assert.equal(compactDiscussionSummary('\n## Cache invalidation\nMore'), 'Cache invalidation')
  assert.equal(compactDiscussionSummary(' \n'), 'Untitled discussion')
})

test('posting from a collapsed reply preview preserves existing replies', () => {
  const preview = reply('preview', 1)
  const optimistic = {
    ...reply('optimistic', 2),
    pending: 'sending' as const,
  }
  const replies = upsertDiscussionReply([], [preview], optimistic)
  assert.deepEqual(
    replies.map(({ id }) => id),
    ['preview', 'optimistic'],
  )
})

test('posting after loading older replies preserves chronological order', () => {
  const current = [reply('one', 1), reply('two', 2), reply('three', 3)]
  const latest = [reply('two', 2), reply('three', 3)]
  const replies = upsertDiscussionReply(
    current,
    latest,
    reply('four', 4),
  )
  assert.deepEqual(
    replies.map(({ id }) => id),
    ['one', 'two', 'three', 'four'],
  )
})

test('expanded replies merge new realtime previews in order', () => {
  const replies = mergeDiscussionReplies(
    [reply('one', 1), reply('two', 2)],
    [reply('two', 2), reply('three', 3)],
  )
  assert.deepEqual(
    replies.map(({ id }) => id),
    ['one', 'two', 'three'],
  )
})

function discussion(id: string, lastActivity: number): RequestDiscussion {
  return {
    author: { handle: 'maya', id: 'user-maya' },
    body_markdown: `Discussion ${id}`,
    created_at_unix: lastActivity,
    id,
    last_activity_position: lastActivity,
    latest_replies: [],
    opened_position: lastActivity,
    reply_count: 0,
    request_id: 'request-1',
    resolved_at_unix: null,
    resolved_by: null,
    status: 'Open',
    unread_count: 0,
  }
}

function reply(id: string, position: number) {
  return {
    author: { handle: 'maya', id: 'user-maya' },
    body_markdown: `Reply ${id}`,
    created_at_unix: position,
    discussion_id: 'one',
    id,
    position,
    reply_to_reply_id: null,
  }
}
