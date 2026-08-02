import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestEvent } from '@/api/types'
import { requestEventBody } from './request-labels'

test('activity describes review transitions without credit data', () => {
  assert.equal(
    requestEventBody(event('ReadyForReview', {
      ReadyForReview: { head_oid: 'a'.repeat(40) },
    })),
    'aaaaaaaaaaaa',
  )
  assert.equal(
    requestEventBody(event('ReturnedToWorking', {
      ReturnedToWorking: {
        head_oid: 'a'.repeat(40),
        reason: 'RevisionPushed',
      },
    })),
    'Branch update invalidated review',
  )
})

function event(kind: RequestEvent['kind'], payload: RequestEvent['payload']) {
  return { kind, payload } as RequestEvent
}
