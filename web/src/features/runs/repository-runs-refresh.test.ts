import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import type { RepoRun } from '@/api/types'
import {
  selectedRunBecameTerminal,
  selectedRunIsUnavailable,
  shouldRefreshSelectedRunDetail,
} from './repository-runs-refresh'

function operationsWith(state: RepoRun['state']) {
  return { runs: [{ id: 'run-1', state }] }
}

describe('repository runs refresh', () => {
  it('refreshes selected detail only while its run is active', () => {
    for (const state of ['queued', 'leased', 'running'] as const) {
      assert.equal(
        shouldRefreshSelectedRunDetail(operationsWith(state), 'run-1'),
        true,
      )
    }
    for (const state of ['succeeded', 'failed', 'canceled', 'lost'] as const) {
      assert.equal(
        shouldRefreshSelectedRunDetail(operationsWith(state), 'run-1'),
        false,
      )
    }
  })

  it('stops refreshing detail for absent or unselected runs', () => {
    assert.equal(
      shouldRefreshSelectedRunDetail(operationsWith('running'), 'run-2'),
      false,
    )
    assert.equal(
      shouldRefreshSelectedRunDetail(operationsWith('running'), null),
      false,
    )
    assert.equal(shouldRefreshSelectedRunDetail(null, 'run-1'), false)
  })

  it('recognizes revoked access and runs removed from the refreshed list', () => {
    assert.equal(selectedRunIsUnavailable(null, 'run-1'), true)
    assert.equal(
      selectedRunIsUnavailable(operationsWith('running'), 'run-2'),
      true,
    )
    assert.equal(
      selectedRunIsUnavailable(operationsWith('running'), 'run-1'),
      false,
    )
    assert.equal(selectedRunIsUnavailable(null, null), false)
  })

  it('requests final detail only on the active-to-terminal transition', () => {
    assert.equal(
      selectedRunBecameTerminal(
        operationsWith('running'),
        operationsWith('succeeded'),
        'run-1',
      ),
      true,
    )
    assert.equal(
      selectedRunBecameTerminal(
        operationsWith('succeeded'),
        operationsWith('succeeded'),
        'run-1',
      ),
      false,
    )
    assert.equal(
      selectedRunBecameTerminal(
        operationsWith('running'),
        operationsWith('running'),
        'run-1',
      ),
      false,
    )
    assert.equal(
      selectedRunBecameTerminal(operationsWith('running'), null, 'run-1'),
      false,
    )
  })
})
