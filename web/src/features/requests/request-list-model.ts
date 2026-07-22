import type { RequestListItem } from '@/api/types'

export function appendRequestPage(
  current: RequestListItem[],
  incoming: RequestListItem[],
) {
  const knownIds = new Set(current.map((request) => request.id))
  const additions = incoming.filter((request) => {
    if (knownIds.has(request.id)) {
      return false
    }
    knownIds.add(request.id)
    return true
  })
  return [...current, ...additions]
}

export function requestCountLabel(count: number, hasMore: boolean) {
  const suffix = count === 1 && !hasMore ? 'request' : 'requests'
  return `${count}${hasMore ? '+' : ''} ${suffix}`
}
