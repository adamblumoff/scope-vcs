import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { getApiConnection } from './client'

const originalApiUrl = process.env.SCOPE_API_INTERNAL_URL
const originalNodeEnv = process.env.NODE_ENV

afterEach(() => {
  restoreEnv('SCOPE_API_INTERNAL_URL', originalApiUrl)
  restoreEnv('NODE_ENV', originalNodeEnv)
})

test('uses the local API only in an explicit development runtime', () => {
  delete process.env.SCOPE_API_INTERNAL_URL
  process.env.NODE_ENV = 'development'

  assert.equal(getApiConnection(), 'http://localhost:8080')
})

test('requires an API URL when the runtime is not explicitly development', () => {
  delete process.env.SCOPE_API_INTERNAL_URL
  delete process.env.NODE_ENV

  assert.throws(
    () => getApiConnection('running the test'),
    /Set SCOPE_API_INTERNAL_URL before running the test\./,
  )
})

function restoreEnv(name: string, value: string | undefined) {
  if (value === undefined) {
    delete process.env[name]
  } else {
    process.env[name] = value
  }
}
