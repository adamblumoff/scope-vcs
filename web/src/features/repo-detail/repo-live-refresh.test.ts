import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import type { RepoChangeEvent } from '@/api/types.generated'
import {
  createRepoRefreshCoordinator,
  parseRepoChangeEvent,
  takeSseMessages,
} from './repo-live-refresh'

const event = (version: number, reason = 'changed', repo_id = 'owner/repo') =>
  ({
    kind: { RepositoryChanged: { reason } },
    repo_id,
    version,
  }) satisfies RepoChangeEvent
const laggedEvent = (repo_id = 'owner/repo') =>
  ({ kind: 'Lagged', repo_id, version: 0 }) satisfies RepoChangeEvent
const discussionEvent = (version: number) =>
  ({
    kind: {
      RequestTimelineChanged: {
        audience: 'Public',
        discussion_id: 'discussion-1',
        request_id: 'request-1',
        through_position: version,
      },
    },
    repo_id: 'owner/repo',
    version,
  }) satisfies RepoChangeEvent
const tick = () => new Promise((resolve) => setImmediate(resolve))

test('SSE parsing validates events and retains partial messages', () => {
  assert.deepEqual(
    parseRepoChangeEvent(
      'event: repo-change\ndata: {"repo_id":"owner/repo","version":2,"kind":{"RepositoryChanged":{"reason":"visibility-changed"}}}',
    ),
    event(2, 'visibility-changed'),
  )
  for (const message of [
    ': keep-alive',
    'event: other\ndata: {}',
    'event: repo-change\ndata: {',
    'event: repo-change\ndata: {"repo_id":1,"version":2,"kind":"Connected"}',
    'event: repo-change\ndata: {"repo_id":"owner/repo","version":2,"kind":{"RequestTimelineChanged":{"request_id":"request-1"}}}',
  ]) assert.equal(parseRepoChangeEvent(message), null)
  assert.deepEqual(takeSseMessages('event: one\n\nevent: two'), {
    messages: ['event: one'],
    rest: 'event: two',
  })
})

test('coordinator ignores stale, connected, and wrong-repo events', async () => {
  let refreshes = 0
  const coordinator = coordinatorFor(async () => { refreshes += 1 }, 2)
  coordinator.onEvent(event(2))
  coordinator.onEvent({ kind: 'Connected', repo_id: 'owner/repo', version: 3 })
  coordinator.onEvent(discussionEvent(3))
  coordinator.onEvent(event(3, 'changed', 'other/repo'))
  await tick()
  assert.equal(refreshes, 0)
})

test('request revisions refresh summaries while discussion activity stays targeted', async () => {
  let refreshes = 0
  const coordinator = coordinatorFor(async () => { refreshes += 1 }, 2)
  coordinator.onEvent(discussionEvent(3))
  await tick()
  assert.equal(refreshes, 0)

  coordinator.onEvent(event(0, 'request-revised'))
  await tick()
  assert.equal(refreshes, 1)
})

test('coordinator coalesces versions received during refresh', async () => {
  const releases: Array<() => void> = []
  let refreshes = 0
  const coordinator = coordinatorFor(() => new Promise<void>((resolve) => {
    refreshes += 1
    releases.push(resolve)
  }))
  coordinator.onEvent(event(2))
  coordinator.onEvent(event(2))
  coordinator.onEvent(event(3))
  assert.equal(refreshes, 1)
  releases.shift()?.()
  await tick()
  assert.equal(refreshes, 2)
  releases.shift()?.()
  await tick()
  coordinator.onEvent(event(3))
  await tick()
  assert.equal(refreshes, 2)
})

test('lagged, unversioned, version-zero, and interrupted streams force refresh', async () => {
  for (const trigger of [
    (value: ReturnType<typeof coordinatorFor>) => value.onEvent(laggedEvent()),
    (value: ReturnType<typeof coordinatorFor>) => value.onEvent(event(0)),
    (value: ReturnType<typeof coordinatorFor>) => value.onStreamInterrupted(),
  ]) {
    let refreshes = 0
    const coordinator = coordinatorFor(async () => { refreshes += 1 }, 5)
    trigger(coordinator)
    await tick()
    assert.equal(refreshes, 1)
  }
  let publicRefreshes = 0
  const publicCoordinator = coordinatorFor(async () => { publicRefreshes += 1 }, 5, false)
  publicCoordinator.onEvent(event(1))
  await tick()
  assert.equal(publicRefreshes, 1)
})

test('failed refresh retries once and stop cancels pending retry', async () => {
  const retries: Array<() => void> = []
  let attempts = 0
  const coordinator = coordinatorFor(async () => {
    attempts += 1
    if (attempts === 1) throw new Error('temporary')
  }, 0, true, (retry) => { retries.push(retry); return () => {} })
  coordinator.onEvent(event(1))
  await tick()
  assert.equal(retries.length, 1)
  retries[0]()
  await tick()
  assert.equal(attempts, 2)

  let cancelled = false
  const stopped = coordinatorFor(async () => { throw new Error('temporary') }, 0, true,
    () => () => { cancelled = true })
  stopped.onEvent(event(1))
  await tick()
  stopped.stop()
  assert.equal(cancelled, true)
})

function coordinatorFor(
  invalidate: () => Promise<unknown>,
  initialVersion = 0,
  versioned = true,
  scheduleRetry = (_retry: () => void) => () => {},
) {
  return createRepoRefreshCoordinator({
    initialVersion,
    invalidate,
    repoId: 'owner/repo',
    scheduleRetry,
    versioned,
  })
}
