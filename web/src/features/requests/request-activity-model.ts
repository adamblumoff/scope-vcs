import type { RequestActivityPage } from './request-discussion-types'

export const MAX_REQUEST_ACTIVITY_EVENTS = 1_000

export async function collectRequestActivityPages(
  after: number,
  loadPage: (after: number) => Promise<RequestActivityPage>,
): Promise<RequestActivityPage> {
  const events: RequestActivityPage['events'] = []
  const seen = new Set<string>()
  let cursor = after
  let throughPosition = after
  while (events.length < MAX_REQUEST_ACTIVITY_EVENTS) {
    const page = await loadPage(cursor)
    for (const event of page.events) {
      if (!seen.has(event.id)) {
        seen.add(event.id)
        events.push(event)
        if (events.length === MAX_REQUEST_ACTIVITY_EVENTS) break
      }
    }
    throughPosition = Math.max(throughPosition, page.through_position)
    if (page.events.length < 100 || page.through_position <= cursor) {
      return {
        events,
        through_position: throughPosition,
      }
    }
    cursor = page.through_position
  }
  return {
    events,
    through_position: throughPosition,
  }
}

export function mergeRequestActivity(
  current: RequestActivityPage,
  incoming: RequestActivityPage,
): RequestActivityPage {
  const byId = new Map(current.events.map((event) => [event.id, event]))
  for (const event of incoming.events) {
    byId.set(event.id, event)
  }
  const events = [...byId.values()].sort(
      (left, right) => left.position - right.position,
    )
  return {
    events: events.slice(-MAX_REQUEST_ACTIVITY_EVENTS),
    through_position: Math.max(
      current.through_position,
      incoming.through_position,
    ),
  }
}
