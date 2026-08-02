import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestEvent, RequestListItem } from '@/api/types'
import { requestCompletionMergeLabel, requestEventBody } from './request-labels'

test('accepted completed rows distinguish merged from mergeable results', () => {
  assert.equal(
    requestCompletionMergeLabel(request('Accepted', 'Completed')),
    'Merged',
  )
  assert.equal(
    requestCompletionMergeLabel(request('Accepted', 'Ready')),
    'Not merged',
  )
})

test('non-accepted completed rows are never described as merged', () => {
  assert.equal(
    requestCompletionMergeLabel(request('Neutral', 'Completed')),
    'Not merged',
  )
  assert.equal(
    requestCompletionMergeLabel(request('Rejected', 'Completed')),
    'Not merged',
  )
})

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
        reason: 'ChangesRequested',
      },
    })),
    'Maintainer requested changes',
  )
})

function event(kind: RequestEvent['kind'], payload: RequestEvent['payload']) {
  return { kind, payload } as RequestEvent
}

function request(
  assessment: RequestListItem['assessment_outcome'],
  mergeability: RequestListItem['mergeability']['status'],
) {
  return {
    assessment_outcome: assessment,
    mergeability: { status: mergeability },
    state: 'Completed',
  } as RequestListItem
}
