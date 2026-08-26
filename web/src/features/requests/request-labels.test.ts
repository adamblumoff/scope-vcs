import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestEvent } from '@/api/types'
import {
  formatRelativeUnix,
  formatUnixDate,
  formatUnixDateUtc,
  requestEventBody,
} from './request-labels'

test('request dates are stable across server and browser time zones', () => {
  assert.equal(formatUnixDate(0), 'Jan 01, 1970, 12:00 AM')
  assert.equal(formatUnixDateUtc(0), 'Jan 01, 1970, 12:00 AM')
  assert.equal(formatUnixDate(null), 'Not set')
})

test('activity describes submission', () => {
  assert.equal(
    requestEventBody(event('Submitted', {
      Submitted: { head_oid: 'a'.repeat(40) },
    })),
    'aaaaaaaaaaaa',
  )
})

function event(kind: RequestEvent['kind'], payload: RequestEvent['payload']) {
  return { kind, payload } as RequestEvent
}

const NOW_SECONDS = Date.UTC(2026, 0, 15, 12, 0, 0) / 1_000

test('relative time uses natural wording inside thirty days', () => {
  const cases: [offset: number, expected: string][] = [
    [0, 'just now'],
    [-59, 'just now'],
    [30, 'just now'],
    [-60, '1 minute ago'],
    [-300, '5 minutes ago'],
    [-3599, '59 minutes ago'],
    [300, 'in 5 minutes'],
    [-3600, '1 hour ago'],
    [-7200, '2 hours ago'],
    [-86399, '23 hours ago'],
    [-86400, 'yesterday'],
    [-86400 * 3, '3 days ago'],
    [86400, 'tomorrow'],
  ]
  for (const [offset, expected] of cases) {
    assert.equal(formatRelativeUnix(NOW_SECONDS + offset, NOW_SECONDS), expected)
  }
})

test('relative time falls back to an absolute date past thirty days', () => {
  const justInside = NOW_SECONDS - (86400 * 30 - 1)
  const justOutside = NOW_SECONDS - 86400 * 30
  assert.equal(formatRelativeUnix(justInside, NOW_SECONDS), '29 days ago')
  assert.equal(
    formatRelativeUnix(justOutside, NOW_SECONDS),
    formatUnixDate(justOutside),
  )
  assert.equal(
    formatRelativeUnix(justOutside, NOW_SECONDS),
    'Dec 16, 2025, 12:00 PM',
  )
})

test('relative time has the same missing value wording as absolute dates', () => {
  assert.equal(formatRelativeUnix(null, NOW_SECONDS), 'Not set')
})

test('relative time defaults its reference point to now', () => {
  assert.equal(formatRelativeUnix(Date.now() / 1000), 'just now')
})
