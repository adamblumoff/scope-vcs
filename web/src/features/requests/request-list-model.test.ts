import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestList, RequestListItem } from '@/api/types'
import {
  appendRequestPage,
  requestCountLabel,
  requestListSnapshotKey,
} from './request-list-model'

test('appendRequestPage preserves order and ignores repeated request ids', () => {
  const first = request('req_1')
  const repeated = request('req_1')
  const second = request('req_2')

  assert.deepEqual(appendRequestPage([first], [repeated, second, second]), [
    first,
    second,
  ])
})

test('requestCountLabel marks partial counts until the final page', () => {
  assert.equal(requestCountLabel(50, true), '50+ requests')
  assert.equal(requestCountLabel(51, false), '51 requests')
  assert.equal(requestCountLabel(1, false), '1 request')
})

test('requestListSnapshotKey changes with refreshed loader data', () => {
  const original = {
    requests: [request('req_1')],
    next_cursor: 'v1:req_1',
  } as RequestList
  const refreshed = {
    requests: [{ ...request('req_1'), state: 'Resolved' }],
    next_cursor: null,
  } as RequestList

  assert.equal(
    requestListSnapshotKey(original),
    requestListSnapshotKey(structuredClone(original)),
  )
  assert.notEqual(requestListSnapshotKey(original), requestListSnapshotKey(refreshed))
})

function request(id: string) {
  return { id } as RequestListItem
}
