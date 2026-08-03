import * as assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'
import { HttpError, loadJson } from './http'

const originalFetch = globalThis.fetch
afterEach(() => { globalThis.fetch = originalFetch })

test('loadJson parses success and preserves request init', async () => {
  let captured: RequestInit | undefined
  globalThis.fetch = async (_url, init) => {
    captured = init
    return jsonResponse({ ok: true }, 200)
  }
  assert.deepEqual(await loadJson('/v1/repos', {
    headers: { authorization: 'Bearer repo-token' },
  }), { ok: true })
  assert.deepEqual(captured?.headers, { authorization: 'Bearer repo-token' })
})

test('loadJson surfaces structured and malformed API errors', async () => {
  globalThis.fetch = async () => jsonResponse({ error: 'access denied' }, 403)
  await assert.rejects(loadJson('/v1/repos/hidden'), hasHttpError(403, 'access denied'))

  globalThis.fetch = async () => new Response('not json', { status: 502 })
  await assert.rejects(loadJson('/v1/repos'), hasHttpError(502, 'request failed: 502'))
})

const jsonResponse = (body: unknown, status: number) => new Response(JSON.stringify(body), {
  headers: { 'content-type': 'application/json' }, status,
})

const hasHttpError = (status: number, message: string) => (error: unknown) =>
  error instanceof HttpError && error.status === status && error.message === message
