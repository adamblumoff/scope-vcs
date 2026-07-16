import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestActivityPage } from './request-discussion-types'
import {
  collectRequestActivityPages,
  MAX_REQUEST_ACTIVITY_EVENTS,
  mergeRequestActivity,
} from './request-activity-model'

test('merges realtime activity without duplicates and advances position', () => {
  const current = page([event('one', 1), event('two', 2)], 2)
  const merged = mergeRequestActivity(
    current,
    page([event('two', 2), event('three', 4)], 5),
  )
  assert.deepEqual(
    merged.events.map(({ id }) => id),
    ['one', 'two', 'three'],
  )
  assert.equal(merged.through_position, 5)
})

test('collects every activity page past the API limit', async () => {
  const firstEvents = Array.from(
    { length: 100 },
    (_, index) => event(`event-${index + 1}`, index + 1),
  )
  const cursors: number[] = []
  const result = await collectRequestActivityPages(0, async (after) => {
    cursors.push(after)
    return after === 0
      ? page(firstEvents, 100)
      : page([event('event-101', 101)], 104)
  })
  assert.deepEqual(cursors, [0, 100])
  assert.equal(result.events.length, 101)
  assert.equal(result.through_position, 104)
})

test('bounds eager activity collection and live merges', async () => {
  let calls = 0
  const collected = await collectRequestActivityPages(0, async (after) => {
    calls += 1
    const events = Array.from(
      { length: 100 },
      (_, index) => event(`event-${after + index + 1}`, after + index + 1),
    )
    return page(events, after + 100)
  })
  assert.equal(calls, 10)
  assert.equal(collected.events.length, MAX_REQUEST_ACTIVITY_EVENTS)

  const merged = mergeRequestActivity(
    collected,
    page([event('event-1001', 1001)], 1001),
  )
  assert.equal(merged.events.length, MAX_REQUEST_ACTIVITY_EVENTS)
  assert.equal(merged.events[0]?.position, 2)
  assert.equal(merged.events.at(-1)?.position, 1001)
})

function page(
  events: RequestActivityPage['events'],
  throughPosition: number,
): RequestActivityPage {
  return {
    events,
    through_position: throughPosition,
  }
}

function event(id: string, position: number) {
  return {
    actor: { handle: 'maya', id: 'user-maya' },
    actor_user_id: 'user-maya',
    created_at_unix: position,
    id,
    kind: 'Started' as const,
    payload: {
      Started: {
        description_markdown: '',
        title: id,
      },
    },
    position,
    request_id: 'request-1',
  }
}
