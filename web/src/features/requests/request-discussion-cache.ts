import { createBoundedCache } from '../../lib/bounded-cache'
import type { DiscussionCollection } from './request-discussion-model'

const MAX_ENTRIES = 8
const MAX_DISCUSSIONS = 500

type CacheEntry = {
  collection: DiscussionCollection
  scrollTop: number
}

const entries = createBoundedCache<string, CacheEntry>({
  maxEntries: MAX_ENTRIES,
})

export function requestDiscussionCacheKey({
  repoId,
  requestId,
}: {
  repoId: string
  requestId: string
}) {
  return [repoId, requestId].join('\0')
}

export function readRequestDiscussionCache(key: string) {
  return entries.get(key)?.collection ?? null
}

export function writeRequestDiscussionCache(
  key: string,
  collection: DiscussionCollection,
) {
  const scrollTop = entries.peek(key)?.scrollTop ?? 0
  entries.set(key, {
    collection: limitCollection(collection),
    scrollTop,
  })
}

export function readRequestDiscussionScroll(key: string) {
  return entries.peek(key)?.scrollTop ?? 0
}

export function writeRequestDiscussionScroll(key: string, scrollTop: number) {
  const entry = entries.peek(key)
  if (entry) entry.scrollTop = scrollTop
}

export function resetRequestDiscussionCache() {
  entries.clear()
}

function limitCollection(collection: DiscussionCollection): DiscussionCollection {
  if (collection.order.length <= MAX_DISCUSSIONS) return collection
  const order = collection.order.slice(-MAX_DISCUSSIONS)
  const ids = new Set(order)
  return {
    ...collection,
    byId: new Map(
      [...collection.byId].filter(([discussionId]) => ids.has(discussionId)),
    ),
    order,
  }
}
