import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  createRunTimeFormatter,
  elapsedDuration,
  formatDuration,
  formatRelativeTime,
  runDisplayState,
  runUnixTimeDate,
} from './run-formatting'

describe('run formatting', () => {
  const now = Date.parse('2026-08-09T21:30:00Z') / 1_000

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

  it('counts a running span up from the shared clock', () => {
    assert.equal(elapsedDuration(100, null, 145), '45s')
    assert.equal(elapsedDuration(100, 130, 999), '30s')
    assert.equal(elapsedDuration(null, null, 145), null)
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

  it('reads relative inside a month', () => {
    assert.equal(formatRelativeTime(now - 5, now), 'just now')
    assert.equal(formatRelativeTime(now - 240, now), '4m ago')
    assert.equal(formatRelativeTime(now - 7_200, now), '2h ago')
    assert.equal(formatRelativeTime(now - 90_000, now), 'yesterday')
    assert.equal(formatRelativeTime(now - 5 * 86_400, now), '5d ago')
  })

  it('falls back to a date past a month', () => {
    assert.equal(formatRelativeTime(now - 60 * 86_400, now), 'Jun 10, 2026')
  })

  it('formats durations for scanning', () => {
    assert.equal(formatDuration(44), '44s')
    assert.equal(formatDuration(184), '3m 04s')
    assert.equal(formatDuration(120), '2m')
    assert.equal(formatDuration(4_320), '1h 12m')
  })
})
