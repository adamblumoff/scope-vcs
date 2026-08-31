import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoChangeEvent } from '@/api/types.generated'
import {
  createRunRefreshCoordinator,
  type RunRefreshReason,
} from './run-live-refresh'

const tick = () => new Promise((resolve) => setImmediate(resolve))

test('filters run events by repository, run, and change kind', async () => {
  const received: RunRefreshReason[][] = []
  const coordinator = createCoordinator(async (reasons) => {
    received.push([...reasons])
  })
  coordinator.onEvent(runEvent('other', 'run-1', 'StatusChanged'))
  coordinator.onEvent(runEvent('repo-1', 'run-2', 'StatusChanged'))
  coordinator.onEvent(runEvent('repo-1', 'run-1', 'LogsAppended'))
  coordinator.onEvent(runEvent('repo-1', 'run-1', 'StatusChanged'))
  await tick()
  assert.deepEqual(received, [['StatusChanged']])
})

test('coalesces events received while a refresh is in flight', async () => {
  const releases: Array<() => void> = []
  const received: RunRefreshReason[][] = []
  const coordinator = createCoordinator((reasons) => new Promise<void>((resolve) => {
    received.push([...reasons])
    releases.push(resolve)
  }), ['Created', 'StatusChanged'])
  coordinator.onEvent(runEvent('repo-1', 'run-1', 'Created'))
  coordinator.onEvent(runEvent('repo-1', 'run-1', 'StatusChanged'))
  coordinator.onEvent(runEvent('repo-1', 'run-1', 'StatusChanged'))
  assert.deepEqual(received, [['Created']])
  releases.shift()?.()
  await tick()
  assert.deepEqual(received, [['Created'], ['StatusChanged']])
  releases.shift()?.()
  await tick()
})

test('connected and lagged events request recovery refreshes', async () => {
  const received: RunRefreshReason[][] = []
  const coordinator = createCoordinator(async (reasons) => {
    received.push([...reasons])
  })
  coordinator.onEvent({
    incarnation_id: 'repoi-repo-1',
    kind: 'Connected',
    repo_id: 'repo-1',
    version: 1,
  })
  await tick()
  coordinator.onEvent({
    incarnation_id: 'repoi-repo-1',
    kind: 'Lagged',
    repo_id: 'repo-1',
    version: 0,
  })
  await tick()
  assert.deepEqual(received, [['Recovery'], ['Recovery']])
})

test('times out a hung refresh and retries without overlapping requests', async () => {
  const scheduled: Array<{ callback: () => void; delay: number }> = []
  const signals: AbortSignal[] = []
  let active = 0
  let maxActive = 0
  const coordinator = createRunRefreshCoordinator({
    acceptedChanges: ['StatusChanged'],
    refresh: (_reasons, signal) => new Promise<void>((_resolve, reject) => {
      signals.push(signal)
      active += 1
      maxActive = Math.max(maxActive, active)
      signal.addEventListener('abort', () => {
        active -= 1
        reject(new Error('aborted'))
      }, { once: true })
    }),
    repoId: 'repo-1',
    runId: 'run-1',
    schedule: (callback, delay) => {
      const task = { callback, delay }
      scheduled.push(task)
      return () => {
        const index = scheduled.indexOf(task)
        if (index >= 0) scheduled.splice(index, 1)
      }
    },
    timeoutMs: 10,
  })
  coordinator.requestRefresh()
  assert.equal(active, 1)
  scheduled.find(({ delay }) => delay === 10)?.callback()
  await tick()
  assert.equal(signals[0]?.aborted, true)
  assert.equal(active, 0)
  scheduled.find(({ delay }) => delay === 2_000)?.callback()
  await tick()
  assert.equal(active, 1)
  assert.equal(maxActive, 1)
  coordinator.stop()
})

test('timeout releases the coordinator when refresh ignores abort', async () => {
  const scheduled: Array<{ callback: () => void; delay: number }> = []
  const signals: AbortSignal[] = []
  let calls = 0
  const coordinator = createRunRefreshCoordinator({
    acceptedChanges: ['StatusChanged'],
    refresh: (_reasons, signal) => {
      calls += 1
      signals.push(signal)
      return new Promise<void>(() => {})
    },
    repoId: 'repo-1',
    runId: 'run-1',
    schedule: (callback, delay) => {
      const task = { callback, delay }
      scheduled.push(task)
      return () => {
        const index = scheduled.indexOf(task)
        if (index >= 0) scheduled.splice(index, 1)
      }
    },
    timeoutMs: 10,
  })
  coordinator.requestRefresh()
  scheduled.find(({ delay }) => delay === 10)?.callback()
  await tick()
  assert.equal(signals[0]?.aborted, true)
  scheduled.find(({ delay }) => delay === 2_000)?.callback()
  await tick()
  assert.equal(calls, 2)
  coordinator.stop()
})

function createCoordinator(
  refresh: Parameters<typeof createRunRefreshCoordinator>[0]['refresh'],
  acceptedChanges: Parameters<typeof createRunRefreshCoordinator>[0]['acceptedChanges'] = [
    'StatusChanged',
  ],
) {
  return createRunRefreshCoordinator({
    acceptedChanges,
    refresh,
    repoId: 'repo-1',
    runId: 'run-1',
    schedule: () => () => {},
  })
}

function runEvent(
  repoId: string,
  runId: string,
  change: 'Created' | 'StatusChanged' | 'LogsAppended',
): RepoChangeEvent {
  return {
    incarnation_id: `repoi-${repoId}`,
    kind: { RunChanged: { change, run_id: runId } },
    repo_id: repoId,
    version: 1,
  }
}
