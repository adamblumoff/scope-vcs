import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  attemptForJob,
  defaultShowGraph,
  latestAttempt,
  mergeStepLogs,
  reconcileAttemptOverrides,
  runCanChange,
  selectInitialStep,
} from './repository-run-detail-model'

function job(overrides: {
  attempts?: Array<{
    id: string
    number: number
    steps: Array<{ index: number; state: string }>
  }>
  key: string
  needs?: string[]
  state: string
}) {
  return {
    attempts: overrides.attempts ?? [],
    job: { key: overrides.key, needs: overrides.needs ?? [], state: overrides.state },
  }
}

describe('repository run detail model', () => {
  it('identifies states that can still change', () => {
    for (const state of ['queued', 'dispatching', 'running'] as const) {
      assert.equal(runCanChange(state), true)
    }
    for (const state of ['succeeded', 'failed', 'canceled', 'lost'] as const) {
      assert.equal(runCanChange(state), false)
    }
  })

  it('selects the first failed step of the first failed job', () => {
    const jobs = [
      job({
        attempts: [{ id: 'a1', number: 1, steps: [{ index: 0, state: 'succeeded' }] }],
        key: 'lint',
        state: 'succeeded',
      }),
      job({
        attempts: [{
          id: 'a2',
          number: 2,
          steps: [
            { index: 0, state: 'succeeded' },
            { index: 1, state: 'failed' },
          ],
        }],
        key: 'backend',
        state: 'failed',
      }),
      job({
        attempts: [{ id: 'a3', number: 3, steps: [{ index: 0, state: 'running' }] }],
        key: 'web',
        state: 'running',
      }),
    ]
    assert.deepEqual(selectInitialStep(jobs), {
      attemptId: 'a2',
      jobKey: 'backend',
      stepIndex: 1,
    })
  })

  it('falls back to the currently running step when nothing failed', () => {
    const jobs = [
      job({
        attempts: [{ id: 'a1', number: 1, steps: [{ index: 0, state: 'succeeded' }] }],
        key: 'lint',
        state: 'succeeded',
      }),
      job({
        attempts: [{
          id: 'a2',
          number: 2,
          steps: [
            { index: 0, state: 'succeeded' },
            { index: 1, state: 'running' },
          ],
        }],
        key: 'backend',
        state: 'running',
      }),
    ]
    assert.deepEqual(selectInitialStep(jobs), {
      attemptId: 'a2',
      jobKey: 'backend',
      stepIndex: 1,
    })
  })

  it('falls back to the last step of the last job when the run is idle', () => {
    const jobs = [
      job({
        attempts: [{ id: 'a1', number: 1, steps: [{ index: 0, state: 'succeeded' }] }],
        key: 'lint',
        state: 'succeeded',
      }),
      job({
        attempts: [{
          id: 'a2',
          number: 2,
          steps: [
            { index: 0, state: 'succeeded' },
            { index: 1, state: 'succeeded' },
          ],
        }],
        key: 'backend',
        state: 'succeeded',
      }),
    ]
    assert.deepEqual(selectInitialStep(jobs), {
      attemptId: 'a2',
      jobKey: 'backend',
      stepIndex: 1,
    })
  })

  it('selects nothing when there are no steps to show', () => {
    assert.equal(selectInitialStep([]), null)
    assert.equal(
      selectInitialStep([job({ key: 'lint', state: 'queued' })]),
      null,
    )
  })

  it('picks the selected step attempt over the switcher and default attempt', () => {
    const target = job({
      attempts: [
        { id: 'a1', number: 1, steps: [{ index: 0, state: 'failed' }] },
        { id: 'a2', number: 2, steps: [{ index: 0, state: 'succeeded' }] },
      ],
      key: 'backend',
      state: 'succeeded',
    })
    assert.equal(attemptForJob(target, {}, null)?.id, 'a2')
    assert.equal(attemptForJob(target, { backend: 'a1' }, null)?.id, 'a1')
    assert.equal(
      attemptForJob(target, { backend: 'a1' }, {
        attemptId: 'a2',
        jobKey: 'backend',
        stepIndex: 0,
      })?.id,
      'a2',
    )
  })

  it('drops attempt overrides that reference retired attempts', () => {
    const jobs = [
      job({ attempts: [{ id: 'new', number: 1, steps: [] }], key: 'backend', state: 'succeeded' }),
    ]
    assert.deepEqual(
      reconcileAttemptOverrides({ backend: 'old', missing: 'x' }, jobs),
      {},
    )
    assert.deepEqual(
      reconcileAttemptOverrides({ backend: 'new' }, jobs),
      { backend: 'new' },
    )
  })

  it('only defaults to the graph once dependencies make the strip hard to scan', () => {
    const independent = [0, 1, 2, 3].map((index) =>
      job({ key: `job-${index}`, state: 'succeeded' }))
    assert.equal(defaultShowGraph(independent), false)

    const dependent = [
      job({ key: 'a', state: 'succeeded' }),
      job({ key: 'b', needs: ['a'], state: 'succeeded' }),
      job({ key: 'c', state: 'succeeded' }),
      job({ key: 'd', state: 'succeeded' }),
    ]
    assert.equal(defaultShowGraph(dependent), true)
    assert.equal(defaultShowGraph(dependent.slice(0, 3)), false)
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

describe('attempt ordering', () => {
  // The run detail response returns attempts newest first, so anything that
  // relies on array position picks the wrong attempt.
  const newestFirst = [
    { id: 'a2', number: 2, steps: [] },
    { id: 'a1', number: 1, steps: [] },
  ]

  it('reads the latest attempt by number, not by position', () => {
    assert.equal(latestAttempt(newestFirst)?.id, 'a2')
    assert.equal(latestAttempt(newestFirst.slice(0, 0))?.id, undefined)
  })

  it('defaults a job to its latest attempt', () => {
    const jobDetail = { attempts: newestFirst, job: { key: 'lint' } }

    assert.equal(attemptForJob(jobDetail, {}, null)?.id, 'a2')
  })
})
