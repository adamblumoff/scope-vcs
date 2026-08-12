import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  createRunTimeFormatter,
  formatRunRunnerSelection,
  runDisplayState,
  runUnixTimeDate,
} from './run-formatting'

describe('run formatting', () => {
  it('states named and mixed runner selection honestly', () => {
    assert.equal(
      formatRunRunnerSelection({ kind: 'named', name: 'linux-one' }),
      'linux-one',
    )
    assert.equal(formatRunRunnerSelection({ kind: 'mixed' }), 'multiple runners')
    assert.equal(formatRunRunnerSelection({ kind: 'any' }), 'any runner')
  })

  it('labels an acknowledged cancellation consistently', () => {
    assert.equal(
      runDisplayState({ cancellation_requested: true, state: 'running' }),
      'canceling',
    )
    assert.equal(
      runDisplayState({ cancellation_requested: false, state: 'running' }),
      'running',
    )
    assert.equal(
      runDisplayState({ cancellation_requested: true, state: 'canceled' }),
      'canceled',
    )
  })

  it('formats the same run instant in the requested timezone', () => {
    const value = Date.parse('2026-08-09T21:30:00Z') / 1_000
    const date = runUnixTimeDate(value)

    assert.equal(date.toISOString(), '2026-08-09T21:30:00.000Z')
    assert.equal(
      createRunTimeFormatter('UTC').format(date),
      'Aug 9, 2026, 9:30 PM',
    )
    assert.equal(
      createRunTimeFormatter('America/Chicago').format(date),
      'Aug 9, 2026, 4:30 PM',
    )
  })
})
