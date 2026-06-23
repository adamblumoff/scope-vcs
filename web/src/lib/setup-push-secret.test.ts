import * as assert from 'node:assert/strict'
import { randomUUID } from 'node:crypto'
import { test } from 'node:test'

import { setupPushSecretKey } from '../api/setup-token-key'
import {
  rememberSetupPushSecret,
  setupPushSecretSnapshot,
  storeSetupPushSecret,
} from './setup-push-secret'

test('setup token snapshot consumes the one-time session secret', () => {
  const repoId = `adam/cache-${randomUUID()}`
  const storage = fakeSessionStorage({
    [setupPushSecretKey(repoId)]: 'old_secret',
  })

  assert.equal(setupPushSecretSnapshot(repoId, storage), 'old_secret')
  assert.equal(storage.getItem(setupPushSecretKey(repoId)), null)
})

test('setup token regeneration replaces a consumed one-time secret', () => {
  const repoId = `adam/cache-${randomUUID()}`
  const storage = fakeSessionStorage({
    [setupPushSecretKey(repoId)]: 'old_secret',
  })

  assert.equal(setupPushSecretSnapshot(repoId, storage), 'old_secret')

  storeSetupPushSecret(repoId, 'new_secret', storage)

  assert.equal(
    setupPushSecretSnapshot(repoId, fakeSessionStorage()),
    'new_secret',
  )
})

test('setup token regeneration without a secret clears cached secret', () => {
  const repoId = `adam/cache-${randomUUID()}`
  const storage = fakeSessionStorage({
    [setupPushSecretKey(repoId)]: 'old_secret',
  })

  rememberSetupPushSecret(repoId, 'old_secret')
  storeSetupPushSecret(repoId, null, storage)

  assert.equal(setupPushSecretSnapshot(repoId, storage), null)
  assert.equal(storage.getItem(setupPushSecretKey(repoId)), null)
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
    setItem(key: string, value: string) {
      values.set(key, value)
    },
  }
}
