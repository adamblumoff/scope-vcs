import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestListItem } from '@/api/types'
import { requestCompletionMergeLabel } from './request-labels'

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
