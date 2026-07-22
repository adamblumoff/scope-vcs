import { createBoundedCache } from '../../lib/bounded-cache'

const sourceScrollPositions = createBoundedCache<string, number>({
  maxEntries: 64,
})

export function readRepositorySourceScroll(key: string | null) {
  if (!key) return 0
  return sourceScrollPositions.peek(key) ?? 0
}

export function writeRepositorySourceScroll(key: string | null, scrollTop: number) {
  if (!key) return
  sourceScrollPositions.set(key, scrollTop)
}

export function resetRepositorySourceScrollCache() {
  sourceScrollPositions.clear()
}
