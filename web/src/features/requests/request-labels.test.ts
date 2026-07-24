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

test('activity describes repeated stake cycles without exposing a balance', () => {
  assert.equal(
    requestEventBody(event('ReadyForReview', {
      ReadyForReview: { head_oid: 'a'.repeat(40), stake_credits: 18 },
    })),
    'aaaaaaaaaaaa · 18 staked',
  )
  assert.equal(
    requestEventBody(event('ReturnedToWorking', {
      ReturnedToWorking: {
        head_oid: 'a'.repeat(40),
        reason: 'ChangesRequested',
        stake_credits: 18,
      },
    })),
    'Maintainer requested changes · 18 refunded',
  )
  assert.equal(
    requestEventBody(event('Settled', {
      Settled: {
        settlement: {
          burned_credits: 0,
          outcome: 'Accepted',
          refunded_credits: 25,
          reward_credits: 25,
          settled_at_unix: 1,
          stake_credits: 25,
        },
      },
    })),
    '25 refunded / 25 reward / 0 burned',
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
