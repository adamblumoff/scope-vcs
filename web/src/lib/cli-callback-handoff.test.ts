import { strict as assert } from 'node:assert'
import test from 'node:test'
import {
  handOffCliCallbackToLocalCli,
  parseCliCallbackHandoffUrl,
} from './cli-callback-handoff'

test('parseCliCallbackHandoffUrl only accepts explicit loopback callbacks', () => {
  for (const host of ['127.0.0.1', 'localhost', '[::1]']) {
    const callback = `http://${host}:61353/scope-cli-callback?request_id=cli_browser_123`
    assert.equal(parseCliCallbackHandoffUrl(callback), callback)
  }
  for (const callback of [
    'https://scopevcs.com/scope-cli-callback?request_id=cli_browser_123',
    'http://example.com:61353/scope-cli-callback?request_id=cli_browser_123',
    'http://127.0.0.1/scope-cli-callback?request_id=cli_browser_123',
    'http://127.0.0.1:61353/other?request_id=cli_browser_123',
  ]) assert.throws(() => parseCliCallbackHandoffUrl(callback))
})

test('handOffCliCallbackToLocalCli uses top-level navigation for valid callbacks', () => {
  const assigned: string[] = []
  const callback =
    'http://127.0.0.1:61353/scope-cli-callback?request_id=cli_browser_123&code=scope_callback_456'

  handOffCliCallbackToLocalCli(callback, {
    assign(url: string) {
      assigned.push(url)
    },
  })

  assert.deepEqual(assigned, [callback])
})
