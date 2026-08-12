import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  mergeStepLogs,
  reconcileExpandedAttempts,
  reconcileExpandedJobs,
  runNeedsPolling,
} from './repository-run-detail-model'

describe('repository run detail model', () => {
  it('polls only while a run can still change', () => {
    for (const state of ['queued', 'dispatching', 'running'] as const) {
      assert.equal(runNeedsPolling(state), true)
    }
    for (const state of ['succeeded', 'failed', 'canceled', 'lost'] as const) {
      assert.equal(runNeedsPolling(state), false)
    }
  })

  it('preserves manual expansion without opening new attempts during polling', () => {
    const expanded = reconcileExpandedAttempts(
      new Set(['old']),
      ['retry', 'current', 'old'],
    )
    assert.deepEqual([...expanded], ['old'])
    assert.deepEqual([...reconcileExpandedAttempts(new Set(), ['new'])], [])
  })

  it('keeps known job expansion without reopening collapsed jobs', () => {
    const jobs = [
      { job: { key: 'backend', state: 'succeeded' }, attempts: [] },
      { job: { key: 'web', state: 'queued' }, attempts: [] },
    ]
    assert.deepEqual([...reconcileExpandedJobs(new Set(), jobs)], [])
    assert.deepEqual(
      [...reconcileExpandedJobs(new Set(['web', 'removed']), jobs)],
      ['web'],
    )
  })

  it('merges incremental logs by stable position', () => {
    assert.deepEqual(mergeStepLogs(
      [{ position: 1, text: 'one' }, { position: 2, text: 'two' }],
      [{ position: 2, text: 'two' }, { position: 3, text: 'three' }],
    ), {
      logs: [
        { position: 1, text: 'one' },
        { position: 2, text: 'two' },
        { position: 3, text: 'three' },
      ],
      truncated: false,
    })
  })

  it('retains only a bounded suffix of selected step logs', () => {
    const text = 'x'.repeat(300 * 1_024)
    assert.deepEqual(mergeStepLogs(
      [{ position: 1, text }],
      [{ position: 2, text }],
    ), {
      logs: [{ position: 2, text }],
      truncated: true,
    })
  })
})
