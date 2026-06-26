import { strict as assert } from 'node:assert'
import test from 'node:test'
import {
  normalizeCliLoginCode,
  parseCompleteBrowserCliLoginInput,
  parseCompleteCliLoginInput,
  parseRevokeCliSessionInput,
} from './cli-login-input'

test('normalizeCliLoginCode accepts spaced and dashed user codes', () => {
  assert.equal(normalizeCliLoginCode(' abcd-1234 '), 'ABCD1234')
  assert.equal(normalizeCliLoginCode('ab cd 12 34'), 'ABCD1234')
})

test('parseCompleteCliLoginInput requires a code', () => {
  assert.throws(() => parseCompleteCliLoginInput({ code: '   ' }))
  assert.deepEqual(parseCompleteCliLoginInput({ code: 'abcd1234' }), {
    code: 'ABCD1234',
  })
})

test('parseCompleteBrowserCliLoginInput requires a browser request id', () => {
  assert.throws(() =>
    parseCompleteBrowserCliLoginInput({ requestId: 'scope_browser_secret' }),
  )
  assert.deepEqual(
    parseCompleteBrowserCliLoginInput({ requestId: ' cli_browser_123 ' }),
    { requestId: 'cli_browser_123' },
  )
})

test('parseRevokeCliSessionInput requires a CLI session id', () => {
  assert.throws(() => parseRevokeCliSessionInput({ sessionId: 'scope_cli_123' }))
  assert.deepEqual(parseRevokeCliSessionInput({ sessionId: ' cli_sess_123 ' }), {
    sessionId: 'cli_sess_123',
  })
})
