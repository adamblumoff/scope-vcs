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
  const byId = new Map(collection.byId)
  const seen = new Set(collection.order)
  const older = [...page.discussions].reverse()
  const added: string[] = []
  for (const discussion of older) {
    const previous = byId.get(discussion.id)
    byId.set(discussion.id, {
      ...discussion,
      initiallyResolved:
        previous?.initiallyResolved ?? discussion.status === 'Resolved',
    })
    if (!seen.has(discussion.id)) {
      seen.add(discussion.id)
      added.push(discussion.id)
    }
  }
  return {
    byId,
    nextCursor: page.next_cursor,
    order: [...added, ...collection.order],
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
    order: [...collection.order.filter((id) => id !== discussion.id), discussion.id],
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
  let order: string[]
  if (found && previousId === discussion.id) {
    order = collection.order
  } else if (found) {
    const withoutOptimistic = {
      ...collection,
      byId,
      order: collection.order.filter((id) => id !== previousId),
    }
    order = insertByOpenedPosition(withoutOptimistic, discussion)
  } else {
    order = insertByOpenedPosition(collection, discussion)
  }
  return {
    ...collection,
    byId,
    order: unique(order),
  }
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

export function patchDiscussionWithoutReordering(
  collection: DiscussionCollection,
  discussion: RequestDiscussion,
): DiscussionCollection {
  return replaceDiscussion(collection, discussion)
}

export function patchDiscussionForFilter(
  collection: DiscussionCollection,
  discussion: RequestDiscussion,
): DiscussionCollection {
  return patchDiscussionWithoutReordering(collection, discussion)
}

export function applyDiscussionChangesWithoutReordering(
  collection: DiscussionCollection,
  discussions: RequestDiscussion[],
  throughPosition: number,
): DiscussionCollection {
  let next = collection
  for (const discussion of discussions) {
    next = patchDiscussionForFilter(next, discussion)
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

export function compactDiscussionSummary(body: string | null) {
  if (!body) return 'Code change'
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
