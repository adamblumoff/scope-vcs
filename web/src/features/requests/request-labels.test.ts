import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestEvent } from '@/api/types'
import { formatUnixDate, requestEventBody } from './request-labels'

test('request dates are stable across server and browser time zones', () => {
  assert.equal(formatUnixDate(0), 'Jan 01, 1970, 12:00 AM')
  assert.equal(formatUnixDate(null), 'Not set')
})

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
