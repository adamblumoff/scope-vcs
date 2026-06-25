import { strict as assert } from 'node:assert'
import test from 'node:test'
import {
  normalizeCliLoginCode,
  parseCompleteCliLoginInput,
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
