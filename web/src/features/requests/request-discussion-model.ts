import type {
  RequestDiscussion,
  RequestDiscussionFilter,
  RequestDiscussionPage,
  RequestDiscussionReplyView,
  RequestDiscussionSort,
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
  for (const discussion of page.discussions) {
    byId.set(discussion.id, {
      ...discussion,
      initiallyResolved: discussion.status === 'Resolved',
    })
  }
  return {
    byId,
    nextCursor: page.next_cursor,
    order: page.discussions.map(({ id }) => id),
    snapshotVersion: page.snapshot_version,
  }
}

export function appendDiscussionPage(
  collection: DiscussionCollection,
  page: RequestDiscussionPage,
): DiscussionCollection {
  const byId = new Map(collection.byId)
  const seen = new Set(collection.order)
  const order = [...collection.order]
  for (const discussion of page.discussions) {
    const previous = byId.get(discussion.id)
    byId.set(discussion.id, {
      ...discussion,
      initiallyResolved:
        previous?.initiallyResolved ?? discussion.status === 'Resolved',
    })
    if (!seen.has(discussion.id)) {
      seen.add(discussion.id)
      order.push(discussion.id)
    }
  }
  return {
    byId,
    nextCursor: page.next_cursor,
    order,
    snapshotVersion: Math.max(
      collection.snapshotVersion,
      page.snapshot_version,
    ),
  }
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
    order: [discussion.id, ...collection.order.filter((id) => id !== discussion.id)],
  }
}

export function replaceDiscussion(
  collection: DiscussionCollection,
  discussion: RequestDiscussionView,
  previousId = discussion.id,
): DiscussionCollection {
  const byId = new Map(collection.byId)
  const previous = byId.get(previousId)
  byId.delete(previousId)
  byId.set(discussion.id, {
    ...discussion,
    initiallyResolved:
      previous?.initiallyResolved ?? discussion.status === 'Resolved',
  })
  const found = collection.order.includes(previousId)
  const order = found
    ? collection.order.map((id) => id === previousId ? discussion.id : id)
    : [...collection.order, discussion.id]
  return {
    ...collection,
    byId,
    order: unique(order),
  }
}

export function patchDiscussionWithoutReordering(
  collection: DiscussionCollection,
  discussion: RequestDiscussion,
): DiscussionCollection {
  return replaceDiscussion(collection, discussion)
}

export function patchDiscussionForFilter(
  collection: DiscussionCollection,
  discussion: RequestDiscussion,
  filter: RequestDiscussionFilter,
): DiscussionCollection {
  if (filter === 'Open' && discussion.status === 'Resolved') {
    return removeDiscussion(collection, discussion.id)
  }
  return patchDiscussionWithoutReordering(collection, discussion)
}

export function applyDiscussionChangesWithoutReordering(
  collection: DiscussionCollection,
  discussions: RequestDiscussion[],
  filter: RequestDiscussionFilter,
  throughPosition: number,
): DiscussionCollection {
  let next = collection
  for (const discussion of discussions) {
    next = patchDiscussionForFilter(next, discussion, filter)
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
  return replaceDiscussion(collection, { ...existing, pending: 'failed' })
}

export function markDiscussionRead(
  collection: DiscussionCollection,
  discussionId: string,
): DiscussionCollection {
  const existing = collection.byId.get(discussionId)
  if (!existing || existing.unread_count === 0) return collection
  return replaceDiscussion(collection, { ...existing, unread_count: 0 })
}

export function orderedDiscussions(collection: DiscussionCollection) {
  return collection.order.flatMap((id) => {
    const discussion = collection.byId.get(id)
    return discussion ? [discussion] : []
  })
}

export function reorderDiscussions(
  collection: DiscussionCollection,
  sort: RequestDiscussionSort,
): DiscussionCollection {
  const order = [...collection.order].sort((leftId, rightId) => {
    const left = collection.byId.get(leftId)
    const right = collection.byId.get(rightId)
    if (!left || !right) return 0
    const leftPosition =
      sort === 'Recent' ? left.last_activity_position : left.opened_position
    const rightPosition =
      sort === 'Recent' ? right.last_activity_position : right.opened_position
    return rightPosition - leftPosition
  })
  return { ...collection, order }
}

export function compactDiscussionSummary(body: string) {
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

function unique(values: string[]) {
  return [...new Set(values)]
}

function removeDiscussion(
  collection: DiscussionCollection,
  discussionId: string,
): DiscussionCollection {
  if (!collection.byId.has(discussionId)) return collection
  const byId = new Map(collection.byId)
  byId.delete(discussionId)
  return {
    ...collection,
    byId,
    order: collection.order.filter((id) => id !== discussionId),
  }
}
