import assert from 'node:assert/strict'
import test from 'node:test'
import { analyticsRouteForId } from './routes'

test('route aliases contain no dynamic route values', () => {
  assert.deepEqual(
    analyticsRouteForId('/$owner/$repo/requests/$requestId/changes'),
    {
      name: 'request_changes',
      path: '/repository/request/changes',
    },
  )
  assert.deepEqual(analyticsRouteForId('/$owner/$repo/_code/'), {
    name: 'repository_code',
    path: '/repository/code',
  })
})

test('unknown routes are rejected instead of falling back to a raw path', () => {
  assert.equal(analyticsRouteForId('/adam/private-repo'), null)
  assert.equal(analyticsRouteForId(undefined), null)
})
