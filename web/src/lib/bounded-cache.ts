export type BoundedCacheOptions<Value> = {
  maxEntries: number
  maxWeight?: number
  weightOf?: (value: Value) => number
}

type CacheEntry<Value> = {
  value: Value
  weight: number
}

export function createBoundedCache<Key, Value>({
  maxEntries,
  maxWeight = Number.POSITIVE_INFINITY,
  weightOf = () => 0,
}: BoundedCacheOptions<Value>) {
  const entries = new Map<Key, CacheEntry<Value>>()
  let totalWeight = 0

  return {
    clear() {
      entries.clear()
      totalWeight = 0
    },
    get(key: Key) {
      const entry = entries.get(key)
      if (!entry) return undefined
      entries.delete(key)
      entries.set(key, entry)
      return entry.value
    },
    peek(key: Key) {
      return entries.get(key)?.value
    },
    set(key: Key, value: Value) {
      const previous = entries.get(key)
      if (previous) {
        totalWeight -= previous.weight
        entries.delete(key)
      }

      const entry = { value, weight: weightOf(value) }
      entries.set(key, entry)
      totalWeight += entry.weight

      while (entries.size > maxEntries || totalWeight > maxWeight) {
        const oldest = entries.entries().next()
        if (oldest.done) break
        entries.delete(oldest.value[0])
        totalWeight -= oldest.value[1].weight
      }
    },
    stats() {
      return { entries: entries.size, totalWeight }
    },
  }
}
