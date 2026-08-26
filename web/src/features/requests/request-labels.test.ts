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

test('relative time reports very recent events as just now', () => {
  assert.equal(formatRelativeUnix(NOW_SECONDS, NOW_SECONDS), 'just now')
  assert.equal(formatRelativeUnix(NOW_SECONDS - 59, NOW_SECONDS), 'just now')
  assert.equal(formatRelativeUnix(NOW_SECONDS + 30, NOW_SECONDS), 'just now')
})

test('relative time reports minutes', () => {
  assert.equal(formatRelativeUnix(NOW_SECONDS - 60, NOW_SECONDS), '1 minute ago')
  assert.equal(formatRelativeUnix(NOW_SECONDS - 300, NOW_SECONDS), '5 minutes ago')
  assert.equal(formatRelativeUnix(NOW_SECONDS - 3599, NOW_SECONDS), '59 minutes ago')
  assert.equal(formatRelativeUnix(NOW_SECONDS + 300, NOW_SECONDS), 'in 5 minutes')
})

test('relative time reports hours', () => {
  assert.equal(formatRelativeUnix(NOW_SECONDS - 3600, NOW_SECONDS), '1 hour ago')
  assert.equal(formatRelativeUnix(NOW_SECONDS - 7200, NOW_SECONDS), '2 hours ago')
  assert.equal(formatRelativeUnix(NOW_SECONDS - 86399, NOW_SECONDS), '23 hours ago')
})

test('relative time reports days with natural wording', () => {
  assert.equal(formatRelativeUnix(NOW_SECONDS - 86400, NOW_SECONDS), 'yesterday')
  assert.equal(formatRelativeUnix(NOW_SECONDS - 86400 * 3, NOW_SECONDS), '3 days ago')
  assert.equal(formatRelativeUnix(NOW_SECONDS + 86400, NOW_SECONDS), 'tomorrow')
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
