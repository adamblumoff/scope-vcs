import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestEvent } from '@/api/types'
import { requestEventBody } from './request-labels'

test('activity describes submission', () => {
  assert.equal(
    requestEventBody(event('Submitted', {
      Submitted: { head_oid: 'a'.repeat(40) },
    })),
    'aaaaaaaaaaaa',
  )
})

function event(kind: RequestEvent['kind'], payload: RequestEvent['payload']) {
  return { kind, payload } as RequestEvent
}
