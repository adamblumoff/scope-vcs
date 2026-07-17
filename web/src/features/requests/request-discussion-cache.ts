import type { DiscussionCollection } from './request-discussion-model'

const MAX_ENTRIES = 8
const MAX_DISCUSSIONS = 500

type CacheEntry = {
  collection: DiscussionCollection
  lastAccessed: number
  scrollTop: number
}

const entries = new Map<string, CacheEntry>()
let accessClock = 0

export function requestDiscussionCacheKey({
  filter,
  repoId,
  requestId,
  sort,
}: {
  filter: string
  repoId: string
  requestId: string
  sort: string
}) {
  return [repoId, requestId, filter, sort].join('\0')
}

export function readRequestDiscussionCache(key: string) {
  const entry = entries.get(key)
  if (!entry) return null
  entry.lastAccessed = nextAccess()
  return entry.collection
}

export function writeRequestDiscussionCache(
  key: string,
  collection: DiscussionCollection,
) {
  const limited = limitCollection(collection)
  entries.set(key, {
    collection: limited,
    lastAccessed: nextAccess(),
    scrollTop: entries.get(key)?.scrollTop ?? 0,
  })
  evictOldEntries(key)
}

export function readRequestDiscussionScroll(key: string) {
  return entries.get(key)?.scrollTop ?? 0
}

export function writeRequestDiscussionScroll(key: string, scrollTop: number) {
  const entry = entries.get(key)
  if (entry) entry.scrollTop = scrollTop
}

export function resetRequestDiscussionCache() {
  entries.clear()
  accessClock = 0
}

export function requestDiscussionCacheStats() {
  return {
    discussions: [...entries.values()].reduce(
      (total, entry) => total + entry.collection.order.length,
      0,
    ),
    entries: entries.size,
  }
}

function limitCollection(collection: DiscussionCollection): DiscussionCollection {
  if (collection.order.length <= MAX_DISCUSSIONS) return collection
  const order = collection.order.slice(0, MAX_DISCUSSIONS)
  const ids = new Set(order)
  return {
    ...collection,
    byId: new Map(
      [...collection.byId].filter(([discussionId]) => ids.has(discussionId)),
    ),
    order,
  }
}

function evictOldEntries(protectedKey: string) {
  while (entries.size > MAX_ENTRIES) {
    let oldest: [string, CacheEntry] | null = null
    for (const entry of entries) {
      if (entry[0] === protectedKey) continue
      if (!oldest || entry[1].lastAccessed < oldest[1].lastAccessed) {
        oldest = entry
      }
    }
    if (!oldest) return
    entries.delete(oldest[0])
  }
}

function nextAccess() {
  accessClock += 1
  return accessClock
}
