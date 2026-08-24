import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { runStatus } from './run-status'

describe('run status', () => {
  it('separates running from succeeded', () => {
    const running = runStatus('running')
    const succeeded = runStatus('succeeded')

    assert.equal(running.tone, 'running')
    assert.equal(running.animated, true)
    assert.equal(succeeded.tone, 'success')
    assert.equal(succeeded.animated, false)
  })

  it('prefers a terminal reason over the raw state', () => {
    assert.equal(
      runStatus('failed', { kind: 'timed-out', step_index: 2 }).label,
      'timed out',
    )
    assert.equal(
      runStatus('failed', { kind: 'step-failed', step_index: 1, exit_code: 2 })
        .label,
      'failed',
    )
  })

  it('groups every waiting state under one tone', () => {
    for (const state of ['pending', 'blocked', 'queued', 'dispatching', 'canceling']) {
      assert.equal(runStatus(state).tone, 'waiting', state)
    }
  })

  it('treats states that never ran as inert', () => {
    assert.equal(runStatus('canceled').tone, 'inert')
    assert.equal(runStatus('skipped').tone, 'inert')
  })
})
