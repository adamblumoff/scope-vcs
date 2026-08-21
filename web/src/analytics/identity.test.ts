import assert from 'node:assert/strict'
import test from 'node:test'
import {
  identityTransition,
  resolveAnalyticsIdentity,
} from './identity'

test('signed-in anonymous visitors identify with the internal Scope user ID', () => {
  assert.deepEqual(identityTransition({
    currentDistinctId: 'anonymous-id',
    isSignedIn: true,
    persistedUserId: undefined,
    scopeUserId: 'scope_usr_123',
  }), {
    kind: 'identify',
    scopeUserId: 'scope_usr_123',
  })
})

test('sign-out resets an identified browser but not a new anonymous visit', () => {
  assert.deepEqual(identityTransition({
    currentDistinctId: 'scope_usr_123',
    isSignedIn: false,
    persistedUserId: 'scope_usr_123',
  }), { kind: 'reset' })
  assert.deepEqual(identityTransition({
    currentDistinctId: 'anonymous-id',
    isSignedIn: false,
    persistedUserId: undefined,
  }), { kind: 'none' })
})

test('sign-out resets a Scope distinct ID even without a persisted user property', () => {
  assert.deepEqual(identityTransition({
    currentDistinctId: 'scope_usr_123',
    isSignedIn: false,
    persistedUserId: undefined,
  }), { kind: 'reset' })
})

test('already identified visitors do not emit a duplicate identify', () => {
  assert.deepEqual(identityTransition({
    currentDistinctId: 'scope_usr_123',
    isSignedIn: true,
    persistedUserId: 'scope_usr_123',
    scopeUserId: 'scope_usr_123',
  }), { kind: 'none' })
})

test('identity lookup failure keeps signed-in pageviews anonymous', async () => {
  const identity = await resolveAnalyticsIdentity(
    'clerk_user_123',
    () => Promise.reject(new Error('identity unavailable')),
  )

  assert.deepEqual(identity, {
    identityKey: 'identified:clerk_user_123',
    scopeUserId: null,
  })
})
