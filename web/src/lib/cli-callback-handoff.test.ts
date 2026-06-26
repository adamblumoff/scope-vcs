import { strict as assert } from 'node:assert'
import test from 'node:test'
import { parseCliCallbackHandoffUrl } from './cli-callback-handoff'

test('parseCliCallbackHandoffUrl accepts loopback CLI callbacks', () => {
  assert.equal(
    parseCliCallbackHandoffUrl(
      'http://127.0.0.1:61353/scope-cli-callback?request_id=cli_browser_123&code=scope_callback_456',
    ),
    'http://127.0.0.1:61353/scope-cli-callback?request_id=cli_browser_123&code=scope_callback_456',
  )
  assert.equal(
    parseCliCallbackHandoffUrl(
      'http://localhost:61353/scope-cli-callback?request_id=cli_browser_123&code=scope_callback_456',
    ),
    'http://localhost:61353/scope-cli-callback?request_id=cli_browser_123&code=scope_callback_456',
  )
  assert.equal(
    parseCliCallbackHandoffUrl(
      'http://[::1]:61353/scope-cli-callback?request_id=cli_browser_123&code=scope_callback_456',
    ),
    'http://[::1]:61353/scope-cli-callback?request_id=cli_browser_123&code=scope_callback_456',
  )
})

test('parseCliCallbackHandoffUrl rejects non-local callbacks', () => {
  assert.throws(() =>
    parseCliCallbackHandoffUrl(
      'https://scopevcs.com/scope-cli-callback?request_id=cli_browser_123',
    ),
  )
  assert.throws(() =>
    parseCliCallbackHandoffUrl(
      'http://example.com:61353/scope-cli-callback?request_id=cli_browser_123',
    ),
  )
  assert.throws(() =>
    parseCliCallbackHandoffUrl(
      'http://127.0.0.1/scope-cli-callback?request_id=cli_browser_123',
    ),
  )
  assert.throws(() =>
    parseCliCallbackHandoffUrl(
      'http://127.0.0.1:61353/other?request_id=cli_browser_123',
    ),
  )
})
