import * as assert from 'node:assert/strict'
import { randomUUID } from 'node:crypto'
import { test } from 'node:test'

import { setupPushSecretKey } from '../../api/setup-token-key'
import {
  rememberSetupPushSecret,
  setupPushSecretSnapshot,
} from './setup-token-cache'

test('setup token regeneration replaces a consumed one-time secret', () => {
  const repoId = `adam/cache-${randomUUID()}`
  const storage = fakeSessionStorage({
    [setupPushSecretKey(repoId)]: 'old_secret',
  })

  assert.equal(setupPushSecretSnapshot(repoId, storage), 'old_secret')
  assert.equal(storage.getItem(setupPushSecretKey(repoId)), null)

  rememberSetupPushSecret(repoId, 'new_secret')

  assert.equal(
    setupPushSecretSnapshot(repoId, fakeSessionStorage()),
    'new_secret',
  )
})

test('setup token regeneration without a secret clears cached secret', () => {
  const repoId = `adam/cache-${randomUUID()}`

  rememberSetupPushSecret(repoId, 'old_secret')
  rememberSetupPushSecret(repoId, null)

  assert.equal(setupPushSecretSnapshot(repoId, fakeSessionStorage()), null)
})

function fakeSessionStorage(initial: Record<string, string> = {}) {
  const values = new Map(Object.entries(initial))

  return {
    getItem(key: string) {
      return values.get(key) ?? null
    },
    removeItem(key: string) {
      values.delete(key)
    },
  }
}
