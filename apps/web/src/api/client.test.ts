import * as assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { HttpError, authHeaders, loadJson } from './client'

const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

test('authHeaders sends bearer auth only when a token is present', () => {
  assert.deepEqual(authHeaders('id-token'), {
    authorization: 'Bearer id-token',
  })
  assert.deepEqual(authHeaders(null), {})
  assert.deepEqual(authHeaders(''), {})
})

test('loadJson returns parsed JSON and preserves request init', async () => {
  let capturedInit: RequestInit | undefined
  globalThis.fetch = async (_url, init) => {
    capturedInit = init
    return new Response(JSON.stringify({ ok: true }), {
      headers: { 'content-type': 'application/json' },
      status: 200,
    })
  }

  await assert.doesNotReject(async () => {
    assert.deepEqual(
      await loadJson('/v1/repos', { headers: authHeaders('repo-token') }),
      { ok: true },
    )
  })
  assert.deepEqual(capturedInit?.headers, {
    authorization: 'Bearer repo-token',
  })
})

test('loadJson throws HttpError with API error text', async () => {
  globalThis.fetch = async () =>
    new Response(JSON.stringify({ error: 'repo is private' }), {
      headers: { 'content-type': 'application/json' },
      status: 403,
    })

  await assert.rejects(
    loadJson('/v1/repos/private'),
    (error) =>
      error instanceof HttpError &&
      error.status === 403 &&
      error.message === 'repo is private',
  )
})

test('loadJson falls back to status text when error JSON is unavailable', async () => {
  globalThis.fetch = async () => new Response('not json', { status: 502 })

  await assert.rejects(
    loadJson('/v1/repos'),
    (error) =>
      error instanceof HttpError &&
      error.status === 502 &&
      error.message === 'request failed: 502',
  )
})

