import assert from 'node:assert/strict'
import test from 'node:test'
import {
  cachedResourceFor,
  resourceErrorMessage,
  startAbortableResourceAttempt,
} from './use-cached-resource'

test('derives idle, loading, and loaded states from identity and cache', () => {
  const cached = { id: 'cached' }

  assert.deepEqual(cachedResourceFor(null, cached), {
    error: null,
    identity: null,
    status: 'idle',
    value: null,
  })
  assert.deepEqual(cachedResourceFor('resource-1', null), {
    error: null,
    identity: 'resource-1',
    status: 'loading',
    value: null,
  })
  assert.deepEqual(cachedResourceFor('resource-1', cached), {
    error: null,
    identity: 'resource-1',
    status: 'loaded',
    value: cached,
  })
})

test('publishes the active attempt result', async () => {
  const loaded: string[] = []
  const failed: unknown[] = []
  const attempt = deferred<string>()
  const cancel = startAbortableResourceAttempt({
    load: () => attempt.promise,
    onFailed: (error) => failed.push(error),
    onLoaded: (value) => loaded.push(value),
  })

  attempt.resolve('ready')
  await attempt.promise

  assert.deepEqual(loaded, ['ready'])
  assert.deepEqual(failed, [])
  cancel()
})

test('aborts and suppresses a late success', async () => {
  const loaded: string[] = []
  const attempt = deferred<string>()
  let signal!: AbortSignal
  const cancel = startAbortableResourceAttempt({
    load: (nextSignal) => {
      signal = nextSignal
      return attempt.promise
    },
    onFailed: () => assert.fail('cancelled attempt must not fail visibly'),
    onLoaded: (value) => loaded.push(value),
  })

  cancel()
  attempt.resolve('stale')
  await attempt.promise

  assert.equal(signal.aborted, true)
  assert.deepEqual(loaded, [])
})

test('suppresses a late rejection even when the loader ignores abort', async () => {
  const failures: unknown[] = []
  const attempt = deferred<string>()
  const cancel = startAbortableResourceAttempt({
    load: () => attempt.promise,
    onFailed: (error) => failures.push(error),
    onLoaded: () => assert.fail('cancelled attempt must not load visibly'),
  })

  cancel()
  attempt.reject(new Error('stale failure'))
  await assert.rejects(attempt.promise, /stale failure/)

  assert.deepEqual(failures, [])
})

test('publishes an active rejection', async () => {
  const failure = new Error('unavailable')
  const failures: unknown[] = []
  const attempt = deferred<string>()
  startAbortableResourceAttempt({
    load: () => attempt.promise,
    onFailed: (error) => failures.push(error),
    onLoaded: () => assert.fail('failed attempt must not load'),
  })

  attempt.reject(failure)
  await assert.rejects(attempt.promise, /unavailable/)

  assert.deepEqual(failures, [failure])
})

test('normalizes errors with a nonblank message or supplied fallback', () => {
  assert.equal(resourceErrorMessage(new Error('request failed'), 'fallback'), 'request failed')
  assert.equal(resourceErrorMessage(new Error('   '), 'fallback'), 'fallback')
  assert.equal(resourceErrorMessage('request failed', 'fallback'), 'fallback')
})

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve
    reject = nextReject
  })
  return { promise, reject, resolve }
}
