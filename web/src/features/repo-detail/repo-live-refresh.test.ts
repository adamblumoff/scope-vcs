import * as assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'
import type { RepoChangeEvent } from '@/api/types.generated'

import {
  createRepoRefreshCoordinator,
} from './repo-live-refresh'
import {
  parseRepoStreamMessage,
  reconnectDelay,
  runRepoEventStream,
  streamRepoEvents,
  takeSseMessages,
  type RepoStreamEnd,
} from './repo-event-stream'

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
const runEvent = (version: number) =>
  ({
    kind: {
      RunChanged: {
        change: 'StatusChanged',
        run_id: 'run-1',
      },
    },
    repo_id: 'owner/repo',
    version,
  }) satisfies RepoChangeEvent
const tick = () => new Promise((resolve) => setImmediate(resolve))
const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

test('SSE parsing returns explicit validated outcomes', () => {
  assert.deepEqual(
    parseRepoStreamMessage(
      'event: repo-change\ndata: {"repo_id":"owner/repo","version":2,"kind":{"RepositoryChanged":{"reason":"visibility-changed"}}}',
    ),
    { type: 'event', event: event(2, 'visibility-changed') },
  )
  assert.deepEqual(
    parseRepoStreamMessage(
      'event: repo-change\ndata: {"repo_id":"owner/repo","version":3,"kind":{"RunChanged":{"run_id":"run-1","change":"LogsAppended"}}}',
    ),
    {
      type: 'event',
      event: {
        kind: { RunChanged: { change: 'LogsAppended', run_id: 'run-1' } },
        repo_id: 'owner/repo',
        version: 3,
      },
    },
  )
  assert.deepEqual(parseRepoStreamMessage(': keep-alive'), { type: 'ignored' })
  assert.deepEqual(parseRepoStreamMessage('event: other\ndata: {}'), { type: 'ignored' })
  assert.deepEqual(
    parseRepoStreamMessage('event: repo-change\ndata: {'),
    { type: 'protocol-error', failureClass: 'json-syntax', issuePath: '/data' },
  )
  const invalid = parseRepoStreamMessage(
    'event: repo-change\ndata: {"repo_id":"owner/repo","version":2,"kind":{"RequestTimelineChanged":{"request_id":"request-1"}}}',
  )
  assert.equal(invalid.type, 'protocol-error')
  assert.equal(invalid.type === 'protocol-error' ? invalid.failureClass : '', 'schema')
  assert.deepEqual(
    parseRepoStreamMessage(
      'event: error\ndata: {"code":"service_unavailable","message":"retry","retryable":true}',
    ),
    {
      type: 'stream-error',
      error: {
        code: 'service_unavailable',
        message: 'retry',
        retryable: true,
      },
    },
  )
  assert.deepEqual(takeSseMessages('event: one\n\nevent: two'), {
    messages: ['event: one'],
    rest: 'event: two',
  })
})

test('reconnect delay doubles with jitter and caps at 30 seconds', () => {
  assert.deepEqual(
    [0, 1, 2, 3, 4, 5].map((attempt) => reconnectDelay(attempt, 0)),
    [2_000, 4_000, 8_000, 16_000, 30_000, 30_000],
  )
  assert.equal(reconnectDelay(0, 1), 2_500)
  assert.equal(reconnectDelay(4, 1), 30_000)
})

test('stream connection errors use the generated public error contract', async () => {
  const connection = {
    clerk_token_template: 'scope_api',
    event_stream_url: 'https://api.scope.test/v1/repos/owner/repo/events',
  }
  globalThis.fetch = async () => new Response(JSON.stringify({
    code: 'service_unavailable',
    message: 'retry later',
    retryable: true,
  }), {
    headers: { 'content-type': 'application/json' },
    status: 503,
  })
  assert.deepEqual(
    await streamRepoEvents(
      connection,
      async () => null,
      () => assert.fail('no event expected'),
      new AbortController().signal,
    ),
    {
      type: 'stream-error',
      error: {
        code: 'service_unavailable',
        message: 'retry later',
        retryable: true,
      },
    },
  )

  globalThis.fetch = async () => new Response(JSON.stringify({ message: 'bad' }), {
    headers: { 'content-type': 'application/json' },
    status: 503,
  })
  const invalid = await streamRepoEvents(
    connection,
    async () => null,
    () => assert.fail('no event expected'),
    new AbortController().signal,
  )
  assert.equal(invalid.type, 'protocol-error')
  assert.equal(invalid.type === 'protocol-error' ? invalid.failureClass : '', 'schema')
})

test('stream decoding handles split CRLF frames before a public stream error', async () => {
  const encoder = new TextEncoder()
  const chunks = [
    'event: repo-change\r',
    '\ndata: {"repo_id":"owner/repo","version":2,"kind":{"RepositoryChanged":{"reason":"changed"}}}\r\n\r\n',
    'event: error\r\ndata: {"code":"forbidden","message":"access changed","retryable":false}\r\n\r\n',
  ]
  globalThis.fetch = async () => new Response(new ReadableStream({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk))
      controller.close()
    },
  }), { headers: { 'content-type': 'text/event-stream; charset=utf-8' } })
  const delivered: RepoChangeEvent[] = []

  const outcome = await streamRepoEvents(
    {
      clerk_token_template: 'scope_api',
      event_stream_url: 'https://api.scope.test/v1/repos/owner/repo/events',
    },
    async () => 'token',
    (change) => delivered.push(change),
    new AbortController().signal,
  )

  assert.deepEqual(delivered, [event(2)])
  assert.deepEqual(outcome, {
    type: 'stream-error',
    error: { code: 'forbidden', message: 'access changed', retryable: false },
  })
})

test('protocol failures cancel response bodies before reconnecting', async () => {
  const encoder = new TextEncoder()
  let malformedBodyCanceled = false
  globalThis.fetch = async () => new Response(new ReadableStream({
    start(controller) {
      controller.enqueue(encoder.encode('event: repo-change\ndata: {\n\n'))
    },
    cancel() {
      malformedBodyCanceled = true
    },
  }), { headers: { 'content-type': 'text/event-stream' } })

  const malformed = await streamRepoEvents(
    {
      clerk_token_template: 'scope_api',
      event_stream_url: 'https://api.scope.test/v1/repos/owner/repo/events',
    },
    async () => null,
    () => assert.fail('no event expected'),
    new AbortController().signal,
  )

  assert.deepEqual(malformed, {
    type: 'protocol-error',
    failureClass: 'json-syntax',
    issuePath: '/data',
  })
  assert.equal(malformedBodyCanceled, true)

  let wrongTypeBodyCanceled = false
  globalThis.fetch = async () => new Response(new ReadableStream({
    cancel() {
      wrongTypeBodyCanceled = true
    },
  }), { headers: { 'content-type': 'text/plain' } })

  const wrongType = await streamRepoEvents(
    {
      clerk_token_template: 'scope_api',
      event_stream_url: 'https://api.scope.test/v1/repos/owner/repo/events',
    },
    async () => null,
    () => assert.fail('no event expected'),
    new AbortController().signal,
  )

  assert.deepEqual(wrongType, {
    type: 'protocol-error',
    failureClass: 'content-type',
  })
  assert.equal(wrongTypeBodyCanceled, true)
})

test('stream recovery follows retryable, protocol, and terminal outcomes', async () => {
  const outcomes: RepoStreamEnd[] = [
    { type: 'protocol-error', failureClass: 'schema', issuePath: '/kind' },
    {
      type: 'stream-error',
      error: { code: 'service_unavailable', message: 'retry', retryable: true },
    },
    {
      type: 'stream-error',
      error: { code: 'forbidden', message: 'stopped', retryable: false },
    },
  ]
  const waits: number[] = []
  const diagnostics: unknown[] = []
  let interruptions = 0
  await runRepoEventStream({
    connect: async () => outcomes.shift()!,
    onDiagnostic: (diagnostic) => diagnostics.push(diagnostic),
    onEvent: () => assert.fail('no event expected'),
    onInterrupted: () => { interruptions += 1 },
    random: () => 0,
    signal: new AbortController().signal,
    wait: async (milliseconds) => { waits.push(milliseconds) },
  })

  assert.equal(interruptions, 3)
  assert.deepEqual(waits, [2_000, 4_000])
  assert.deepEqual(diagnostics, [
    { type: 'protocol-error', failureClass: 'schema', issuePath: '/kind' },
  ])
})

test('third transport failure records once and a healthy event resets the run', async () => {
  const outcomes: RepoStreamEnd[] = [
    { type: 'transport' },
    { type: 'transport' },
    { type: 'transport' },
    { type: 'transport' },
    { type: 'transport' },
    {
      type: 'stream-error',
      error: { code: 'not_found', message: 'gone', retryable: false },
    },
  ]
  let connection = 0
  const diagnostics: unknown[] = []
  const waits: number[] = []
  await runRepoEventStream({
    connect: async (deliver) => {
      connection += 1
      if (connection === 4) deliver(event(10))
      return outcomes.shift()!
    },
    onDiagnostic: (diagnostic) => diagnostics.push(diagnostic),
    onEvent: () => {},
    onInterrupted: () => {},
    random: () => 0,
    signal: new AbortController().signal,
    wait: async (milliseconds) => { waits.push(milliseconds) },
  })

  assert.deepEqual(diagnostics, [{ type: 'transport', consecutiveFailures: 3 }])
  assert.deepEqual(waits, [2_000, 4_000, 8_000, 2_000, 4_000])
})

test('an aborted stream stops without refresh, retry, or diagnostics', async () => {
  const controller = new AbortController()
  controller.abort()
  let calls = 0
  await runRepoEventStream({
    connect: async () => { calls += 1; return { type: 'transport' } },
    onDiagnostic: () => { calls += 1 },
    onEvent: () => { calls += 1 },
    onInterrupted: () => { calls += 1 },
    signal: controller.signal,
    wait: async () => { calls += 1 },
  })
  assert.equal(calls, 0)
})

test('coordinator ignores stale, connected, and wrong-repo events', async () => {
  let refreshes = 0
  const coordinator = coordinatorFor(async () => { refreshes += 1 }, 2)
  coordinator.onEvent(event(2))
  coordinator.onEvent({ kind: 'Connected', repo_id: 'owner/repo', version: 3 })
  coordinator.onEvent(discussionEvent(3))
  coordinator.onEvent(runEvent(3))
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
