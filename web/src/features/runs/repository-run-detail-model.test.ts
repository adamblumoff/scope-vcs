import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  defaultSelectedStep,
  defaultSelectedJob,
  mergeStepLogs,
  reconcileAutomaticStepSelection,
  reconcileExpandedAttempts,
  reconcileExpandedJobs,
  runNeedsPolling,
} from './repository-run-detail-model'

describe('repository run detail model', () => {
  it('polls only while a run can still change', () => {
    for (const state of ['queued', 'leased', 'running'] as const) {
      assert.equal(runNeedsPolling(state), true)
    }
    for (const state of ['succeeded', 'failed', 'canceled', 'lost'] as const) {
      assert.equal(runNeedsPolling(state), false)
    }
  })

  it('opens the newest attempt initially without exposing an ordinal', () => {
    assert.deepEqual(
      [...reconcileExpandedAttempts(new Set(), [], ['new', 'old'])],
      ['new'],
    )
  })

  it('preserves expansion choices and opens a newly inserted retry', () => {
    const expanded = reconcileExpandedAttempts(
      new Set(['old']),
      ['current', 'old'],
      ['retry', 'current', 'old'],
    )
    assert.deepEqual([...expanded], ['old', 'retry'])
  })

  it('keeps known job expansion and falls back to the first job', () => {
    const jobs = [
      { job: { key: 'backend', state: 'succeeded' }, attempts: [] },
      { job: { key: 'web', state: 'queued' }, attempts: [] },
    ]
    assert.deepEqual([...reconcileExpandedJobs(new Set(), jobs)], ['backend'])
    assert.deepEqual(
      [...reconcileExpandedJobs(new Set(['web', 'removed']), jobs)],
      ['web'],
    )
    assert.equal(defaultSelectedJob(jobs)?.job.key, 'web')
  })

  it('selects the active or failed step before completed work', () => {
    assert.equal(defaultSelectedStep([
      { index: 0, state: 'succeeded' },
      { index: 1, state: 'running' },
      { index: 2, state: 'pending' },
    ]), 1)
    assert.equal(defaultSelectedStep([
      { index: 0, state: 'succeeded' },
      { index: 1, state: 'failed' },
      { index: 2, state: 'skipped' },
    ]), 1)
    assert.equal(defaultSelectedStep([
      { index: 0, state: 'succeeded' },
      { index: 1, state: 'lost' },
      { index: 2, state: 'skipped' },
    ]), 1)
  })

  it('advances automatic selection while leaving user selection to the controller', () => {
    assert.deepEqual(reconcileAutomaticStepSelection(
      { attemptId: 'attempt', jobKey: 'checks', stepIndex: 0 },
      [{
        id: 'attempt',
        steps: [
          { index: 0, state: 'succeeded' },
          { index: 1, state: 'running' },
        ],
      }],
    ), { attemptId: 'attempt', jobKey: 'checks', stepIndex: 1 })
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
