import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { RunTimestamp } from './run-timestamp'

describe('run timestamp', () => {
  it('uses a deterministic UTC value for server rendering', () => {
    const value = Date.parse('2026-08-09T21:30:00Z') / 1_000
    const html = renderToStaticMarkup(createElement(RunTimestamp, { value }))

    assert.equal(
      html,
      '<time dateTime="2026-08-09T21:30:00.000Z">Aug 9, 2026, 9:30 PM</time>',
    )
  })
})
