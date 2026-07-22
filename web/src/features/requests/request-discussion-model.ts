import type {
  RequestDiscussion,
  RequestDiscussionPage,
  RequestDiscussionReplyView,
  RequestDiscussionView,
} from './request-discussion-types'

export type DiscussionCollection = {
  byId: ReadonlyMap<string, RequestDiscussionView>
  nextCursor: string | null
  order: string[]
  snapshotVersion: number
}

export function collectionFromPage(
  page: RequestDiscussionPage,
): DiscussionCollection {
  const byId = new Map<string, RequestDiscussionView>()
  const discussions = [...page.discussions].reverse()
  for (const discussion of discussions) {
    byId.set(discussion.id, {
      ...discussion,
      initiallyResolved: discussion.status === 'Resolved',
    })
  }
  return {
    byId,
    nextCursor: page.next_cursor,
    order: discussions.map(({ id }) => id),
    snapshotVersion: page.snapshot_version,
  }
}

export function appendDiscussionPage(
  collection: DiscussionCollection,
  page: RequestDiscussionPage,
): DiscussionCollection {
  let next = collection
  const older = [...page.discussions].reverse()
  for (const discussion of older) {
    next = mergeDiscussion(next, discussion)
  }
  return {
    ...next,
    nextCursor: page.next_cursor,
    snapshotVersion: Math.max(next.snapshotVersion, page.snapshot_version),
  }
}

export function mergeRefreshedDiscussionPage(
  collection: DiscussionCollection,
  page: RequestDiscussionPage,
  authoritative: boolean,
): DiscussionCollection {
  if (!authoritative) {
    let next = collection
    for (const discussion of [...page.discussions].reverse()) {
      next = mergeDiscussion(next, discussion)
    }
    return next
  }

  let next: DiscussionCollection = {
    ...collectionFromPage(page),
    snapshotVersion: Math.max(collection.snapshotVersion, page.snapshot_version),
  }
  const refreshedIds = new Set(page.discussions.map(({ id }) => id))
  const acceptedClientIds = new Set(
    page.discussions.flatMap((discussion) => {
      const optimistic = collection.byId.get(discussion.client_discussion_id)
      return optimistic?.pending !== undefined &&
        optimistic.author.id === discussion.author.id
        ? [discussion.client_discussion_id]
        : []
    }),
  )
  for (const discussion of page.discussions) {
    const current = collection.byId.get(discussion.id)
    if (!current) continue
    next = mergeDiscussion(
      next,
      discussion.last_activity_position < current.last_activity_position
        ? current
        : preserveDiscussionUi(current, discussion),
    )
  }
  for (const discussionId of collection.order) {
    const current = collection.byId.get(discussionId)
    if (
      current &&
      !refreshedIds.has(discussionId) &&
      !acceptedClientIds.has(discussionId) &&
      current.pending !== undefined
    ) {
      next = mergeDiscussion(next, current)
    }
  }
  return next
}

export function insertOptimisticDiscussion(
  collection: DiscussionCollection,
  discussion: RequestDiscussionView,
): DiscussionCollection {
  const byId = new Map(collection.byId)
  byId.set(discussion.id, discussion)
  return {
    ...collection,
    byId,
    order: [...collection.order.filter((id) => id !== discussion.id), discussion.id],
  }
}

export function mergeDiscussion(
  collection: DiscussionCollection,
  discussion: RequestDiscussionView,
): DiscussionCollection {
  const optimisticId = discussion.client_discussion_id
  const optimistic = collection.byId.get(optimisticId)
  if (
    optimisticId !== discussion.id &&
    optimistic?.pending !== undefined &&
    optimistic.author.id === discussion.author.id
  ) {
    return reconcileDiscussionMutation(collection, discussion, optimisticId)
  }
  const previous = collection.byId.get(discussion.id)
  const acknowledgesOptimistic =
    previous?.pending !== undefined && discussion.pending === undefined
  if (
    previous &&
    !acknowledgesOptimistic &&
    discussion.last_activity_position < previous.last_activity_position
  ) {
    return collection
  }
  const byId = new Map(collection.byId)
  const merged = preserveDiscussionUi(previous, discussion)
  if (acknowledgesOptimistic) delete merged.pending
  byId.set(discussion.id, merged)
  const order = previous
    ? collection.order
    : insertByOpenedPosition(collection, discussion)
  return {
    ...collection,
    byId,
    order: unique(order),
  }
}

export function reconcileDiscussionMutation(
  collection: DiscussionCollection,
  discussion: RequestDiscussion,
  optimisticId = discussion.id,
): DiscussionCollection {
  if (optimisticId === discussion.id) {
    return mergeDiscussion(collection, discussion)
  }
  const optimistic = collection.byId.get(optimisticId)
  const current = collection.byId.get(discussion.id)
  const acceptIncoming =
    !current ||
    discussion.last_activity_position >= current.last_activity_position
  const byId = new Map(collection.byId)
  byId.delete(optimisticId)
  let order = collection.order.filter((id) => id !== optimisticId)
  if (acceptIncoming) {
    const merged = preserveDiscussionUi(current ?? optimistic, discussion)
    delete merged.pending
    byId.set(discussion.id, merged)
  }
  if (!order.includes(discussion.id)) {
    order = insertByOpenedPosition(
      { ...collection, byId, order },
      byId.get(discussion.id) ?? discussion,
    )
  }
  return { ...collection, byId, order: unique(order) }
}

function insertByOpenedPosition(
  collection: DiscussionCollection,
  discussion: RequestDiscussionView,
) {
  const index = collection.order.findIndex((id) => {
    const existing = collection.byId.get(id)
    return existing && existing.opened_position > discussion.opened_position
  })
  if (index === -1) return [...collection.order, discussion.id]
  return [
    ...collection.order.slice(0, index),
    discussion.id,
    ...collection.order.slice(index),
  ]
}

export function applyDiscussionChanges(
  collection: DiscussionCollection,
  discussions: RequestDiscussion[],
  throughPosition: number,
): DiscussionCollection {
  let next = collection
  for (const discussion of discussions) {
    next = mergeDiscussion(next, discussion)
  }
  return {
    ...next,
    snapshotVersion: Math.max(next.snapshotVersion, throughPosition),
  }
}

export function markDiscussionFailed(
  collection: DiscussionCollection,
  discussionId: string,
): DiscussionCollection {
  const existing = collection.byId.get(discussionId)
  if (!existing) return collection
  return mergeDiscussion(collection, { ...existing, pending: 'failed' })
}

export function markDiscussionRead(
  collection: DiscussionCollection,
  discussionId: string,
): DiscussionCollection {
  const existing = collection.byId.get(discussionId)
  if (!existing || existing.unread_count === 0) return collection
  return mergeDiscussion(collection, { ...existing, unread_count: 0 })
}

export function orderedDiscussions(collection: DiscussionCollection) {
  return collection.order.flatMap((id) => {
    const discussion = collection.byId.get(id)
    return discussion ? [discussion] : []
  })
}

export function compactDiscussionSummary(body: string | null) {
  if (!body) return 'Update'
  return body
    .split('\n')
    .map((line) => line.trim().replace(/^#{1,6}\s+/, ''))
    .find(Boolean) ?? 'Untitled discussion'
}

export function upsertDiscussionReply(
  current: RequestDiscussionReplyView[],
  latest: RequestDiscussionReplyView[],
  reply: RequestDiscussionReplyView,
) {
  const byId = new Map(
    mergeDiscussionReplies(current, latest).map((existing) => [
      existing.id,
      existing,
    ]),
  )
  byId.set(reply.id, reply)
  return [...byId.values()].sort(
    (left, right) => left.position - right.position,
  )
}

export function mergeDiscussionReplies(
  current: RequestDiscussionReplyView[],
  latest: RequestDiscussionReplyView[],
) {
  const byId = new Map(
    current.map((existing) => [existing.id, existing]),
  )
  for (const existing of latest) {
    byId.set(existing.id, existing)
  }
  return [...byId.values()].sort(
    (left, right) => left.position - right.position,
  )
}

export function directDiscussionReplies(
  replies: RequestDiscussionReplyView[],
  parentReplyId: string | null,
) {
  return replies.filter(
    (reply) => reply.reply_to_reply_id === parentReplyId,
  )
}

function unique(values: string[]) {
  return [...new Set(values)]
}

function preserveDiscussionUi(
  current: RequestDiscussionView | undefined,
  incoming: RequestDiscussionView,
): RequestDiscussionView {
  const merged: RequestDiscussionView = {
    ...incoming,
    initiallyResolved:
      current?.initiallyResolved ?? incoming.status === 'Resolved',
  }
  if (current?.expanded !== undefined && merged.expanded === undefined) {
    merged.expanded = current.expanded
  }
  if (current?.pending !== undefined && merged.pending === undefined) {
    merged.pending = current.pending
  }
  return merged
}
