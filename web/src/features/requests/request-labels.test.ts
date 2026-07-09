import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import type { RequestSummary } from '@/api/types'
import {
  formatUnixDate,
  normalizedBody,
  resolutionOptionsFor,
  settlementPreviewFor,
  settlementPreviewText,
} from './request-labels'

describe('request labels', () => {
  it('mirrors request settlement preview math', () => {
    assert.deepEqual(settlementPreviewFor(10, 'Accepted'), {
      burnedCredits: 0,
      refundedCredits: 10,
      rewardCredits: 5,
      stakeCredits: 10,
    })
    assert.deepEqual(settlementPreviewFor(11, 'UsefulNotMerged'), {
      burnedCredits: 0,
      refundedCredits: 11,
      rewardCredits: 2,
      stakeCredits: 11,
    })
    assert.deepEqual(settlementPreviewFor(11, 'Duplicate'), {
      burnedCredits: 6,
      refundedCredits: 5,
      rewardCredits: 0,
      stakeCredits: 11,
    })
    assert.equal(
      settlementPreviewText(settlementPreviewFor(10, 'LowQuality')),
      '0 refunded / 0 reward / 10 burned',
    )
  })

  it('keeps accepted merge-only and hides abandoned unless response was requested', () => {
    assert.deepEqual(
      resolutionOptionsFor(requestWithState('Submitted')).map(
        (option) => option.disposition,
      ),
      [
        'UsefulNotMerged',
        'HiddenContext',
        'NotAligned',
        'Duplicate',
        'LowQuality',
      ],
    )
    assert.deepEqual(
      resolutionOptionsFor(requestWithState('NeedsResponse')).map(
        (option) => option.disposition,
      ),
      [
        'UsefulNotMerged',
        'HiddenContext',
        'NotAligned',
        'Duplicate',
        'Abandoned',
        'LowQuality',
      ],
    )
  })

  it('keeps display helpers explicit about empty values', () => {
    assert.notEqual(formatUnixDate(0), 'Not set')
    assert.equal(formatUnixDate(null), 'Not set')
    assert.equal(normalizedBody('  hello  '), 'hello')
    assert.equal(normalizedBody('   '), null)
  })
})

function requestWithState(state: RequestSummary['state']): RequestSummary {
  return {
    author_role: 'Public',
    author_user_id: 'user_public',
    editor_user_ids: [],
    base_audience: 'Public',
    base_main_oid: 'a'.repeat(40),
    created_at_unix: 1,
    disposition: null,
    head_oid: 'b'.repeat(40),
    id: 'req_1',
    mergeability: {
      current_main_oid: 'a'.repeat(40),
      reason: null,
      request_head_oid: 'b'.repeat(40),
      status: 'Ready',
    },
    permissions: {
      can_comment: true,
      can_delete: true,
      can_invite_editor: true,
      can_mark_needs_response: true,
      can_merge: true,
      can_pull_branch: true,
      can_push_branch: true,
      can_resolve: true,
      can_respond: false,
    },
    request_ref: 'refs/scope/requests/req_1',
    resolved_at_unix: null,
    settlement: null,
    stake_credits: 10,
    state,
    target_branch: 'main',
    title: 'Example request',
    updated_at_unix: 2,
  }
}
