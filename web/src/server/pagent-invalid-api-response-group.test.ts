import * as assert from 'node:assert/strict'
import { test } from 'node:test'
import { InvalidApiResponseError } from '../api/http'
import { invalidApiResponseGroup } from './pagent-invalid-api-response-group'

test('keeps readable invalid response groups within the Pagent limit', () => {
  const error = new InvalidApiResponseError(
    'GET',
    '/v1/repos',
    200,
    'text/html; charset=utf-8',
  )

  assert.equal(
    invalidApiResponseGroup(error),
    'GET:/v1/repos:200:text/html; charset=utf-8',
  )
})

test('hashes oversized invalid response groups deterministically', () => {
  const contentType = `multipart/form-data; boundary=${'x'.repeat(170)}`
  const error = new InvalidApiResponseError(
    'GET',
    '/v1/repos/{owner}/{repo}/requests/{request_id}/revisions/{revision_number}/files/{file_path}',
    200,
    contentType,
  )
  const group = invalidApiResponseGroup(error)

  assert.match(group, /^sha256:[a-f0-9]{64}$/)
  assert.ok(group.length <= 200)
  assert.equal(group, invalidApiResponseGroup(error))
  assert.notEqual(
    group,
    invalidApiResponseGroup(new InvalidApiResponseError(
      error.requestMethod,
      error.requestPath,
      error.status,
      `${contentType}y`,
    )),
  )
})
