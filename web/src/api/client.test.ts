import * as assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'
import { HttpError, InvalidApiResponseError, loadJson } from './http'

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
  globalThis.fetch = async () => jsonResponse({ message: 'repo is private' }, 403)
  await assert.rejects(loadJson('/v1/repos/private'), hasHttpError(403, 'repo is private'))

  globalThis.fetch = async () => new Response('not json', { status: 502 })
  await assert.rejects(loadJson('/v1/repos'), hasHttpError(502, 'request failed: 502'))
})

test('loadJson rejects malformed successful responses with safe diagnostics', async () => {
  globalThis.fetch = async () => new Response('<h1>secret response</h1>', {
    headers: { 'content-type': 'text/html; charset=utf-8' },
    status: 200,
  })

  await assert.rejects(
    loadJson('https://api.scope.test/v1/repos?token=secret', { method: 'post' }),
    (error: unknown) => error instanceof InvalidApiResponseError &&
      error.requestMethod === 'POST' &&
      error.requestPath === '/v1/repos' &&
      error.status === 200 &&
      error.contentType === 'text/html; charset=utf-8' &&
      error.message ===
        'POST /v1/repos returned invalid JSON (200, text/html; charset=utf-8)' &&
      !error.message.includes('secret'),
  )
})

test('loadJson rejects an empty JSON response unless the status allows no content', async () => {
  globalThis.fetch = async () => new Response(null, { status: 200 })
  await assert.rejects(
    loadJson('/v1/repos'),
    (error: unknown) => error instanceof InvalidApiResponseError &&
      error.contentType === null &&
      error.message ===
        'GET /v1/repos returned invalid JSON (200, unknown content type)',
  )

  globalThis.fetch = async () => new Response(null, { status: 204 })
  assert.equal(await loadJson<void>('/v1/cli/sessions/session_1', {
    method: 'DELETE',
  }), undefined)
})

test('loadJson keeps the opaque server reference with a diagnostic error', async () => {
  globalThis.fetch = async () => jsonResponse({
    message: 'Scope hit an internal error.',
    error_reference: 'err_0123456789abcdef0123456789abcdef',
  }, 500)

  await assert.rejects(
    loadJson('/v1/repos'),
    hasHttpError(
      500,
      'Scope hit an internal error. (reference: err_0123456789abcdef0123456789abcdef)',
      'err_0123456789abcdef0123456789abcdef',
    ),
  )
})

const jsonResponse = (body: unknown, status: number) => new Response(JSON.stringify(body), {
  headers: { 'content-type': 'application/json' }, status,
})

const hasHttpError = (status: number, message: string, errorReference?: string) =>
  (error: unknown) => error instanceof HttpError &&
    error.status === status &&
    error.message === message &&
    error.errorReference === errorReference
