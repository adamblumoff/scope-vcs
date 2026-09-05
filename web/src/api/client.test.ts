import * as assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'
import {
  HttpError,
  InvalidApiResponseError,
  loadJson,
  noContent,
  setInvalidApiResponseObserver,
} from './http'
import { apiValidators, type ApiValidator } from './validators.generated'

const originalFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = originalFetch
  setInvalidApiResponseObserver(undefined)
})

test('loadJson validates a successful response and preserves request init', async () => {
  let captured: RequestInit | undefined
  globalThis.fetch = async (_url, init) => {
    captured = init
    return jsonResponse({ ok: true }, 200)
  }

  assert.deepEqual(
    await loadJson('/v1/repos', okValidator, {
      headers: { authorization: 'Bearer repo-token' },
    }),
    { ok: true },
  )
  assert.deepEqual(captured?.headers, { authorization: 'Bearer repo-token' })
})

test('loadJson validates structured API errors', async () => {
  globalThis.fetch = async () => jsonResponse({
    code: 'forbidden',
    message: 'repo is private',
    retryable: false,
  }, 403)

  await assert.rejects(
    loadJson('/v1/repos/private', okValidator),
    hasHttpError(403, 'repo is private'),
  )
})

test('loadJson rejects malformed API error bodies', async () => {
  let observed: InvalidApiResponseError | undefined
  setInvalidApiResponseObserver((error) => { observed = error })
  globalThis.fetch = async () => jsonResponse({ message: 'missing code' }, 502)

  await assert.rejects(
    loadJson('/v1/repos', okValidator),
    (error: unknown) => error instanceof InvalidApiResponseError &&
      error.failureClass === 'schema' &&
      error.status === 502,
  )
  assert.equal(observed?.failureClass, 'schema')
})

test('loadJson rejects the wrong media type without exposing the body', async () => {
  let observed: InvalidApiResponseError | undefined
  setInvalidApiResponseObserver((error) => { observed = error })
  globalThis.fetch = async () => new Response('<h1>secret response</h1>', {
    headers: { 'content-type': 'text/html; charset=utf-8' },
    status: 200,
  })

  await assert.rejects(
    loadJson('https://api.scope.test/v1/repos?token=secret', okValidator, {
      method: 'post',
    }),
    (error: unknown) => error instanceof InvalidApiResponseError &&
      error.requestMethod === 'POST' &&
      error.requestPath === '/v1/repos' &&
      error.status === 200 &&
      error.contentType === 'text/html; charset=utf-8' &&
      error.failureClass === 'content-type' &&
      !error.message.includes('secret'),
  )
  assert.equal(observed?.requestPath, '/v1/repos')
})

test('loadJson preserves its error when the observer fails', async () => {
  setInvalidApiResponseObserver(() => { throw new Error('observer failed') })
  globalThis.fetch = async () => new Response('not json', {
    headers: { 'content-type': 'application/json' },
    status: 200,
  })

  await assert.rejects(
    loadJson('/v1/repos', okValidator),
    (error: unknown) => error instanceof InvalidApiResponseError &&
      error.failureClass === 'json-syntax',
  )
})

test('loadJson replaces sensitive and dynamic path segments with the API template', async () => {
  globalThis.fetch = async () => new Response('not json', {
    headers: { 'content-type': 'application/json' },
    status: 200,
  })

  await assert.rejects(
    loadJson('/v1/repository-invites/invite-bearer-secret/accept', okValidator),
    (error: unknown) => error instanceof InvalidApiResponseError &&
      error.requestPath === '/v1/repository-invites/{token}/accept' &&
      !error.message.includes('invite-bearer-secret'),
  )
  await assert.rejects(
    loadJson('/v1/repos/acme/widgets/requests/request_123', okValidator),
    (error: unknown) => error instanceof InvalidApiResponseError &&
      error.requestPath === '/v1/repos/{owner}/{repo}/requests/{request_id}' &&
      !error.message.includes('acme') &&
      !error.message.includes('request_123'),
  )
})

test('loadJson requires explicit no-content handling', async () => {
  globalThis.fetch = async () => new Response(null, { status: 200 })
  await assert.rejects(
    loadJson('/v1/repos', okValidator),
    (error: unknown) => error instanceof InvalidApiResponseError &&
      error.failureClass === 'content-type',
  )

  globalThis.fetch = async () => new Response(null, { status: 204 })
  assert.equal(
    await loadJson('/v1/cli/sessions/session_1', noContent, { method: 'DELETE' }),
    undefined,
  )
  await assert.rejects(
    loadJson('/v1/repos', okValidator),
    (error: unknown) => error instanceof InvalidApiResponseError &&
      error.failureClass === 'unexpected-no-content',
  )
})

test('loadJson keeps the opaque server reference with a validated error', async () => {
  globalThis.fetch = async () => jsonResponse({
    code: 'internal',
    message: 'Scope hit an internal error.',
    error_reference: 'err_0123456789abcdef0123456789abcdef',
    retryable: false,
  }, 500)

  await assert.rejects(
    loadJson('/v1/repos', okValidator),
    hasHttpError(
      500,
      'Scope hit an internal error. (reference: err_0123456789abcdef0123456789abcdef)',
      'err_0123456789abcdef0123456789abcdef',
    ),
  )
})

test('generated validators expose one bounded issue path', async () => {
  globalThis.fetch = async () => jsonResponse({
    code: 'internal',
    message: 'Scope hit an internal error.',
  }, 500)

  await assert.rejects(
    loadJson('/v1/repos', okValidator),
    (error: unknown) => error instanceof InvalidApiResponseError &&
      error.failureClass === 'schema' &&
      error.issuePath === '/retryable',
  )
  assert.equal(apiValidators.ErrorResponse({
    code: 'internal',
    message: 'Scope hit an internal error.',
    retryable: false,
  }), true)
})

const okValidator = withIssue(
  (value: unknown): value is { ok: boolean } =>
    value !== null &&
    typeof value === 'object' &&
    'ok' in value &&
    typeof value.ok === 'boolean',
)

function withIssue<T>(validate: (value: unknown) => value is T): ApiValidator<T> {
  const validator: ApiValidator<T> = (value: unknown): value is T => {
    const valid = validate(value)
    validator.errors = valid
      ? null
      : [{ instancePath: '/ok', keyword: 'type', message: 'must be boolean' }]
    return valid
  }
  validator.errors = null
  return validator
}

const jsonResponse = (body: unknown, status: number) => new Response(JSON.stringify(body), {
  headers: { 'content-type': 'application/json' },
  status,
})

const hasHttpError = (status: number, message: string, errorReference?: string) =>
  (error: unknown) => error instanceof HttpError &&
    error.status === status &&
    error.message === message &&
    error.errorReference === errorReference

test('loadJson cancels an oversized response stream before consuming further chunks', async () => {
  let pulls = 0
  let cancelled = false
  globalThis.fetch = async () => new Response(new ReadableStream({
    pull(controller) {
      pulls++
      controller.enqueue(new TextEncoder().encode('x'.repeat(100)))
    },
    cancel() { cancelled = true },
  }), { headers: { 'content-type': 'application/json' } })
  await assert.rejects(loadJson('/v1/references', okValidator, {}, 150), InvalidApiResponseError)
  assert.equal(cancelled, true)
  assert.ok(pulls <= 3)
})

test('loadJson accepts a valid response within the streaming byte limit', async () => {
  globalThis.fetch = async () => jsonResponse({ ok: true }, 200)
  assert.deepEqual(await loadJson('/v1/references', okValidator, {}, 100), { ok: true })
})
