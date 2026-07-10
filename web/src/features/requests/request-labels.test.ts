import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import type { RequestSummary } from '@/api/types'
import {
  formatUnixDate,
  normalizedBody,
  resolutionOptionsFor,
  requestStatusLabel,
  settlementPreviewFor,
  settlementPreviewText,
} from './request-labels'

const request = (state: RequestSummary['state']) => ({
  state,
  disposition: null,
  merge_settlement_preview: {
    burned_credits: 0, refunded_credits: 10, reward_credits: 5, stake_credits: 10,
  },
  resolution_options: [
    {
      disposition: 'Duplicate',
      settlement: {
        burned_credits: 6, refunded_credits: 5, reward_credits: 0, stake_credits: 11,
      },
    },
    ...(state === 'NeedsResponse' ? [{
      disposition: 'Abandoned' as const,
      settlement: {
        burned_credits: 10, refunded_credits: 0, reward_credits: 0, stake_credits: 10,
      },
    }] : []),
    {
      disposition: 'LowQuality',
      settlement: {
        burned_credits: 10, refunded_credits: 0, reward_credits: 0, stake_credits: 10,
      },
    },
  ],
} as RequestSummary)

describe('request labels', () => {
  it('renders server-owned settlement previews', () => {
    assert.deepEqual(settlementPreviewFor(request('Submitted'), 'Accepted'), {
      burnedCredits: 0, refundedCredits: 10, rewardCredits: 5, stakeCredits: 10,
    })
    assert.deepEqual(settlementPreviewFor(request('Submitted'), 'Duplicate'), {
      burnedCredits: 6, refundedCredits: 5, rewardCredits: 0, stakeCredits: 11,
    })
    assert.equal(
      settlementPreviewText(settlementPreviewFor(request('Submitted'), 'LowQuality')),
      '0 refunded / 0 reward / 10 burned',
    )
  })

  it('only offers abandoned after a contributor response was requested', () => {
    const options = (state: RequestSummary['state']) =>
      resolutionOptionsFor(request(state)).map(({ disposition }) => disposition)
    assert(!options('Submitted').includes('Abandoned'))
    assert(options('NeedsResponse').includes('Abandoned'))
  })

  it('keeps empty-value display behavior explicit', () => {
    assert.notEqual(formatUnixDate(0), 'Not set')
    assert.equal(formatUnixDate(null), 'Not set')
    assert.equal(normalizedBody('  hello  '), 'hello')
    assert.equal(normalizedBody('   '), null)
    assert.equal(
      requestStatusLabel({ ...request('Resolved'), disposition: 'Accepted' }),
      'Accepted',
    )
  })
})
